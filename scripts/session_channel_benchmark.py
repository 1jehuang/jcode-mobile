#!/usr/bin/env python3
"""Artificial offline session-channel experiment, NOT an authenticated service.

Only AF_UNIX socketpair loopback is used, never a listener, external network,
credentials, or production code. State survives reconnects within this process,
not process crashes. Polling reuses a connection so no handshake/TLS savings are
claimed. Bytes are application framing + JSON, not TCP/IP or HTTP overhead.
Both modes use the same event readiness offsets and per-direction injected
transport delay. Latencies use actual monotonic readiness/observation times.
Poll responses batch ready events, while stream events use individual frames.
The delay is per message, not a bandwidth model. Logical channel scheduling
only isolates preparation, not an already-writing frame or shared congestion.
Scheduling/OS noise is real, but the workload and delay model are artificial.

Run: python3 -B scripts/session_channel_benchmark.py
Test: python3 -B scripts/session_channel_benchmark_test.py
"""

import argparse
import asyncio
from collections import deque
import json
import math
import socket
import statistics
import struct
import time

MAX_FRAME = 16 * 1024


class ProtocolError(ValueError):
    pass


class SnapshotRequired(ProtocolError):
    """The client must explicitly install a snapshot before resuming."""


def encode_frame(message):
    try:
        body = json.dumps(message, separators=(",", ":"), sort_keys=True,
                          allow_nan=False).encode("utf-8")
    except (TypeError, ValueError, UnicodeError, RecursionError) as exc:
        raise ProtocolError("invalid JSON value") from exc
    if not isinstance(message, dict) or not 0 < len(body) <= MAX_FRAME:
        raise ProtocolError("frame must be a bounded JSON object")
    return struct.pack("!I", len(body)) + body


async def read_frame(reader):
    try:
        header = await reader.readexactly(4)
    except asyncio.IncompleteReadError as exc:
        if not exc.partial:
            return None  # orderly shutdown between frames
        raise ProtocolError("truncated frame header") from exc
    size, = struct.unpack("!I", header)
    if not 0 < size <= MAX_FRAME:
        raise ProtocolError("invalid or oversized frame length")
    try:
        body = await reader.readexactly(size)
        value = json.loads(body.decode("utf-8"),
                           parse_constant=lambda value: (_ for _ in ()).throw(
                               ValueError("non-finite JSON")))
    except (asyncio.IncompleteReadError, UnicodeError, ValueError,
            RecursionError) as exc:
        raise ProtocolError("malformed or truncated frame") from exc
    if not isinstance(value, dict):
        raise ProtocolError("frame must be an object")
    return value


class Session:
    """Single-event-loop in-memory session with bounded replay and command IDs.

    Command IDs are retained for the epoch, up to command_limit. Once full new
    IDs are rejected rather than evicted and accidentally re-executed. A new
    epoch is a new Session. No exactly-once guarantee across a process crash.
    """

    def __init__(self, session="demo", epoch="epoch-1", replay_limit=64,
                 command_limit=128):
        if replay_limit < 1 or command_limit < 1:
            raise ValueError("limits must be positive")
        self.session, self.epoch = session, epoch
        self.events = deque(maxlen=replay_limit)
        self.sequence = 0
        self.commands = {}
        self.command_limit = command_limit

    def publish(self, payload):
        event = dict(session=self.session, epoch=self.epoch,
                     sequence=self.sequence + 1, payload=payload)
        # Validate before mutating and detach from caller-owned objects.
        event = json.loads(encode_frame(event)[4:])
        self.sequence += 1
        self.events.append(event)
        return json.loads(json.dumps(event))

    def resume(self, session, epoch, sequence):
        if session != self.session or epoch != self.epoch:
            raise SnapshotRequired("session-or-epoch-mismatch: snapshot required")
        if type(sequence) is not int or sequence < 0 or sequence > self.sequence:
            raise SnapshotRequired("invalid-resume-sequence: snapshot required")
        oldest = self.events[0]["sequence"] if self.events else self.sequence + 1
        if sequence < oldest - 1:
            raise SnapshotRequired("resume-too-old: snapshot required")
        return json.loads(json.dumps([event for event in self.events
                                      if event["sequence"] > sequence]))

    def snapshot(self):
        # Prototype snapshot is just a cursor plus bounded retained history,
        # not a production materialized application state.
        return json.loads(json.dumps(dict(session=self.session, epoch=self.epoch,
                                         sequence=self.sequence, retained=list(self.events))))

    def receive_command(self, command_id, payload):
        if not isinstance(command_id, str) or not command_id or len(command_id) > 128:
            raise ProtocolError("invalid command id")
        canonical = encode_frame(dict(payload=payload))[4:]
        previous = self.commands.get(command_id)
        if previous is not None:
            if previous["payload"] != canonical:
                raise ProtocolError("command-id-payload-conflict")
            return dict(command_id=command_id, ack=previous["ack"], duplicate=True)
        if len(self.commands) >= self.command_limit:
            raise ProtocolError("command-capacity-exhausted")
        self.commands[command_id] = dict(payload=canonical, ack="received")
        return dict(command_id=command_id, ack="received", duplicate=False)

    def complete_command(self, command_id):
        self.commands[command_id]["ack"] = "completed"
        return dict(command_id=command_id, ack="completed")


class Cursor:
    def __init__(self, session, epoch, sequence=0):
        self.session, self.epoch, self.sequence = session, epoch, sequence
        self.snapshot_required = False

    def observe(self, event):
        if self.snapshot_required:
            raise SnapshotRequired("snapshot required before observing more events")
        if event.get("session") != self.session or event.get("epoch") != self.epoch:
            self.snapshot_required = True
            raise SnapshotRequired("session-or-epoch-mismatch: snapshot required")
        sequence = event.get("sequence")
        if type(sequence) is not int or sequence != self.sequence + 1:
            self.snapshot_required = True
            raise SnapshotRequired("sequence-gap-or-duplicate: snapshot required")
        self.sequence = sequence

    def install_snapshot(self, snapshot):
        if type(snapshot.get("sequence")) is not int or snapshot["sequence"] < 0:
            raise ProtocolError("invalid snapshot cursor")
        self.session = snapshot["session"]
        self.epoch = snapshot["epoch"]
        self.sequence = snapshot["sequence"]
        self.snapshot_required = False


class Endpoint:
    def __init__(self, reader, writer):
        self.reader, self.writer = reader, writer
        self.bytes_sent = 0
        self.frames_sent = 0
        self.lock = asyncio.Lock()

    async def send(self, value, delay=0):
        # Delay before writer ownership, so an unready history item cannot
        # monopolize the logical command channel. A transmitted frame itself
        # is non-preemptible, as it would be on a real multiplexed connection.
        frame = encode_frame(value)
        await asyncio.sleep(delay)
        async with self.lock:
            self.writer.write(frame)
            await self.writer.drain()
            self.bytes_sent += len(frame)
            self.frames_sent += 1

    async def receive(self):
        return await read_frame(self.reader)

    async def close(self):
        self.writer.close()
        await self.writer.wait_closed()


async def loopback_pair():
    left, right = socket.socketpair()
    left.setblocking(False)
    right.setblocking(False)
    try:
        ar, aw = await asyncio.open_connection(sock=left)
        br, bw = await asyncio.open_connection(sock=right)
    except BaseException:
        left.close()
        right.close()
        raise
    return Endpoint(ar, aw), Endpoint(br, bw)


class LogicalChannels:
    """Independent bounded preparation tasks feeding one framed stream.

    Slow history preparation does not run in the command handler. This does
    not remove transport-level head-of-line blocking or model congestion.
    Tasks must be joined/cancelled before closing the stream.
    """

    def __init__(self, endpoint, pending_limit=128):
        self.endpoint = endpoint
        self.pending_limit = pending_limit
        self.tasks = set()
        self.errors = []
        self.closed = False

    def schedule(self, channel, payload, ready=None):
        if self.closed or len(self.tasks) >= self.pending_limit:
            raise ProtocolError("channel closed or pending capacity exhausted")

        async def deliver():
            if ready is not None:
                await ready.wait()
            await self.endpoint.send(dict(channel=channel, payload=payload))

        task = asyncio.create_task(deliver())
        self.tasks.add(task)

        def finished(done):
            self.tasks.discard(done)
            if not done.cancelled():
                error = done.exception()
                if error is not None and not self.errors:
                    self.errors.append(error)
                    self.closed = True

        task.add_done_callback(finished)
        return task

    async def close(self):
        self.closed = True
        pending = list(self.tasks)
        for task in pending:
            if not task.done():
                task.cancel()
        await asyncio.gather(*pending, return_exceptions=True)
        self.tasks.clear()
        await self.endpoint.close()
        if self.errors:
            raise self.errors[0]


def validate_parameters(events, interval, poll_interval, delay):
    if type(events) is not int or not 1 <= events <= 64:
        raise ValueError("events must be between 1 and 64")
    if not all(math.isfinite(value) for value in (interval, poll_interval, delay)):
        raise ValueError("timings must be finite")
    if interval <= 0 or poll_interval <= 0 or delay < 0:
        raise ValueError("intervals must be positive and delay nonnegative")


async def benchmark_mode(mode, events=12, interval=0.012,
                         poll_interval=0.04, delay=0.002):
    validate_parameters(events, interval, poll_interval, delay)
    if mode not in ("polling", "stream"):
        raise ValueError("unknown mode")
    client, server = await loopback_pair()
    session = Session()
    cursor = Cursor(session.session, session.epoch)
    changed = asyncio.Event()
    ready_times = {}
    observations = []
    trace = []
    requests = 0
    start = time.perf_counter()

    async def produce():
        for index in range(events):
            offset = (index + 1) * interval
            await asyncio.sleep(max(0, start + offset - time.perf_counter()))
            ready = time.perf_counter()
            event = session.publish(dict(index=index))
            ready_times[event["sequence"]] = ready
            trace.append(dict(kind="ready", sequence=event["sequence"], at=ready))
            changed.set()

    async def serve():
        request = await server.receive()
        if request is None:
            return
        if mode == "stream":
            while True:
                replay = session.resume(request["session"], request["epoch"],
                                        request["sequence"])
                if not replay:
                    changed.clear()
                    await changed.wait()
                    continue
                for event in replay:
                    await server.send(dict(events=[event]), delay)
                    request["sequence"] = event["sequence"]
                if request["sequence"] == events:
                    return
        else:
            while request is not None:
                replay = session.resume(request["session"], request["epoch"],
                                        request["sequence"])
                await server.send(dict(events=replay), delay)
                if replay and replay[-1]["sequence"] == events:
                    return
                request = await server.receive()

    producer = asyncio.create_task(produce())
    serving = asyncio.create_task(serve())
    try:
        next_poll = start
        while cursor.sequence < events:
            if mode == "polling" or requests == 0:
                if mode == "polling":
                    await asyncio.sleep(max(0, next_poll - time.perf_counter()))
                    next_poll = max(next_poll + poll_interval,
                                    time.perf_counter() + poll_interval)
                await client.send(dict(kind=mode, session=cursor.session,
                                       epoch=cursor.epoch, sequence=cursor.sequence), delay)
                requests += 1
            response = await client.receive()
            if response is None:
                raise ProtocolError("unexpected stream shutdown")
            for event in response["events"]:
                observed = time.perf_counter()
                cursor.observe(event)
                ready = ready_times[event["sequence"]]
                assert observed >= ready  # causality, never a speed threshold
                observations.append((observed - ready) * 1000)
                trace.append(dict(kind="observed", sequence=cursor.sequence, at=observed))
        await producer
        await serving
        assert cursor.sequence == events and len(observations) == events
        return dict(mode=mode, events=events, requests=requests,
                    frames=client.frames_sent + server.frames_sent,
                    application_bytes=client.bytes_sent + server.bytes_sent,
                    readiness_to_observation_ms=dict(
                        mean=round(statistics.mean(observations), 3),
                        median=round(statistics.median(observations), 3),
                        max=round(max(observations), 3)),
                    samples_ms=[round(value, 3) for value in observations],
                    causal_trace_ms=[dict(item, at=round((item["at"] - start) * 1000, 3))
                                     for item in trace],
                    causal_assertions="all events observed once, in order, after readiness")
    finally:
        for task in (producer, serving):
            if not task.done():
                task.cancel()
        await asyncio.gather(producer, serving, return_exceptions=True)
        await client.close()
        await server.close()


async def run_benchmark(events=12, interval=0.012, poll_interval=0.04, delay=0.002):
    validate_parameters(events, interval, poll_interval, delay)
    results = [await benchmark_mode(mode, events, interval, poll_interval, delay)
               for mode in ("polling", "stream")]
    return dict(experiment="ARTIFICIAL offline AF_UNIX loopback, not deployable or authenticated",
                limitations="No TLS/HTTP, network congestion, durable storage, auth, or production claims. Polling reuses its socket. Bytes count JSON plus 4-byte framing in both directions.",
                inputs=dict(events=events, readiness_interval_ms=interval * 1000,
                            poll_interval_ms=poll_interval * 1000,
                            one_way_transport_delay_ms=delay * 1000),
                results=results)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--events", type=int, default=12)
    parser.add_argument("--interval-ms", type=float, default=12)
    parser.add_argument("--poll-ms", type=float, default=40)
    parser.add_argument("--delay-ms", type=float, default=2)
    args = parser.parse_args()
    try:
        report = asyncio.run(run_benchmark(args.events, args.interval_ms / 1000,
                                          args.poll_ms / 1000, args.delay_ms / 1000))
    except ValueError as exc:
        parser.error(str(exc))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Offline tests. Run: python3 -B scripts/session_channel_benchmark_test.py"""

import asyncio
import json
import struct
import unittest

from session_channel_benchmark import (
    MAX_FRAME, Cursor, LogicalChannels, ProtocolError, Session,
    SnapshotRequired, encode_frame, loopback_pair, read_frame, run_benchmark,
)


class SessionTests(unittest.TestCase):
    def test_dedup_conflict_received_and_completed_are_distinct(self):
        session = Session(command_limit=1)
        first = session.receive_command("one", {"action": "run", "arg": 1})
        self.assertEqual(first["ack"], "received")
        self.assertFalse(first["duplicate"])
        retry = session.receive_command("one", {"arg": 1, "action": "run"})
        self.assertTrue(retry["duplicate"])
        self.assertEqual(retry["ack"], "received")
        with self.assertRaisesRegex(ProtocolError, "payload-conflict"):
            session.receive_command("one", {"action": "erase"})
        self.assertEqual(session.commands["one"]["ack"], "received")
        self.assertEqual(session.complete_command("one")["ack"], "completed")
        self.assertEqual(session.receive_command("one", {"arg": 1, "action": "run"})["ack"],
                         "completed")
        with self.assertRaisesRegex(ProtocolError, "capacity"):
            session.receive_command("two", {})
        self.assertEqual(len(session.commands), 1)

    def test_bounded_replay_requires_explicit_snapshot(self):
        session = Session(replay_limit=2)
        for value in range(4):
            session.publish(value)
        self.assertEqual(len(session.events), 2)
        with self.assertRaisesRegex(SnapshotRequired, "resume-too-old"):
            session.resume("demo", "epoch-1", 1)
        self.assertEqual([e["sequence"] for e in session.resume("demo", "epoch-1", 2)], [3, 4])
        cursor = Cursor("demo", "epoch-1")
        cursor.install_snapshot(session.snapshot())
        self.assertEqual(cursor.sequence, 4)
        cursor.observe(session.publish("after snapshot"))
        self.assertEqual(cursor.sequence, 5)

    def test_wrong_session_epoch_gap_duplicate_and_invalid_resume(self):
        session = Session()
        one, two = session.publish(1), session.publish(2)
        cursor = Cursor("demo", "epoch-1")
        with self.assertRaisesRegex(SnapshotRequired, "gap"):
            cursor.observe(two)
        self.assertEqual(cursor.sequence, 0)
        with self.assertRaisesRegex(SnapshotRequired, "snapshot required"):
            cursor.observe(one)
        cursor.install_snapshot(dict(session="demo", epoch="epoch-1", sequence=0))
        cursor.observe(one)
        with self.assertRaises(SnapshotRequired):
            cursor.observe(one)
        for changed in (dict(two, epoch="new"), dict(two, session="other"),
                        dict(two, sequence=True)):
            cursor = Cursor("demo", "epoch-1", sequence=1)
            with self.subTest(changed=changed), self.assertRaises(SnapshotRequired):
                cursor.observe(changed)
        for session_id, epoch, seq in (("other", "epoch-1", 0), ("demo", "new", 0),
                                       ("demo", "epoch-1", 99), ("demo", "epoch-1", -1),
                                       ("demo", "epoch-1", True)):
            with self.subTest(seq=seq, epoch=epoch), self.assertRaises(SnapshotRequired):
                session.resume(session_id, epoch, seq)

    def test_oversized_publish_is_not_committed(self):
        session = Session()
        with self.assertRaises(ProtocolError):
            session.publish("x" * MAX_FRAME)
        self.assertEqual(session.sequence, 0)
        self.assertFalse(session.events)

    def test_replay_is_isolated_from_mutating_callers(self):
        session = Session()
        payload = {"nested": [1]}
        published = session.publish(payload)
        payload["nested"].append(2)
        published["sequence"] = 99
        replay = session.resume("demo", "epoch-1", 0)
        replay[0]["payload"]["nested"].append(3)
        session.snapshot()["retained"][0]["sequence"] = 100
        self.assertEqual(session.resume("demo", "epoch-1", 0)[0]["sequence"], 1)
        self.assertEqual(session.events[0]["payload"], {"nested": [1]})

    def test_epoch_change_requires_snapshot_before_new_events(self):
        cursor = Cursor("demo", "old", sequence=50)
        new_session = Session(epoch="new")
        with self.assertRaisesRegex(SnapshotRequired, "epoch-mismatch"):
            cursor.observe(new_session.publish("new epoch"))
        self.assertTrue(cursor.snapshot_required)
        cursor.install_snapshot(new_session.snapshot())
        cursor.observe(new_session.publish("after snapshot"))
        self.assertEqual(cursor.sequence, 2)
        self.assertEqual(cursor.epoch, "new")

    def test_invalid_outbound_frames(self):
        for value in ([], {"x": "x" * MAX_FRAME}, {"x": float("nan")}, {"x": object()}):
            with self.subTest(value=type(value)), self.assertRaises(ProtocolError):
                encode_frame(value)


class AsyncTests(unittest.IsolatedAsyncioTestCase):
    async def test_exact_frame_limit_is_accepted(self):
        empty_size = len(encode_frame({"x": ""})) - 4
        wire = encode_frame({"x": "x" * (MAX_FRAME - empty_size)})
        self.assertEqual(len(wire), MAX_FRAME + 4)
        reader = asyncio.StreamReader()
        reader.feed_data(wire)
        reader.feed_eof()
        self.assertEqual(len((await read_frame(reader))["x"]), MAX_FRAME - empty_size)
        with self.assertRaises(ProtocolError):
            encode_frame({"x": "x" * (MAX_FRAME - empty_size + 1)})

    async def test_malformed_oversized_truncated_and_non_object_frames(self):
        invalid = [b"\x00", struct.pack("!I", 0), struct.pack("!I", MAX_FRAME + 1),
                   struct.pack("!I", 4) + b"{}", struct.pack("!I", 1) + b"\xff"]
        for body in (b"{", b"[]", b"null", b'{"x":NaN}', b'{"x":Infinity}'):
            invalid.append(struct.pack("!I", len(body)) + body)
        for wire in invalid:
            with self.subTest(wire=wire):
                reader = asyncio.StreamReader()
                reader.feed_data(wire)
                reader.feed_eof()
                with self.assertRaises(ProtocolError):
                    await read_frame(reader)

    async def test_oversized_header_rejected_before_waiting_for_body(self):
        reader = asyncio.StreamReader()
        reader.feed_data(struct.pack("!I", MAX_FRAME + 1))
        with self.assertRaisesRegex(ProtocolError, "oversized"):
            await asyncio.wait_for(read_frame(reader), 2)

    async def test_fragmented_and_coalesced_frames(self):
        reader = asyncio.StreamReader()
        first, second = encode_frame({"a": 1}), encode_frame({"b": 2})
        pending = asyncio.create_task(read_frame(reader))
        reader.feed_data(first[:2])
        await asyncio.sleep(0)
        self.assertFalse(pending.done())
        reader.feed_data(first[2:] + second)
        reader.feed_eof()
        self.assertEqual(await pending, {"a": 1})
        self.assertEqual(await read_frame(reader), {"b": 2})
        self.assertIsNone(await read_frame(reader))

    async def test_reconnect_replay_over_new_stream_and_command_retry(self):
        session = Session()
        cursor = Cursor("demo", "epoch-1")
        for index in range(3):
            session.publish(index)
        # Close the first stream after only the first event is observed.
        client, server = await loopback_pair()
        try:
            await server.send(session.resume("demo", "epoch-1", 0)[0])
            cursor.observe(await client.receive())
            session.receive_command("retry-on-reconnect", {"run": True})
        finally:
            await server.close()
            await client.close()
        session.publish(3)
        client, server = await loopback_pair()
        try:
            await client.send(dict(session=cursor.session, epoch=cursor.epoch,
                                   sequence=cursor.sequence))
            request = await server.receive()
            replay = session.resume(**request)
            self.assertEqual([e["sequence"] for e in replay], [2, 3, 4])
            for event in replay:
                await server.send(event)
                cursor.observe(await client.receive())
            self.assertEqual(cursor.sequence, 4)
            self.assertTrue(session.receive_command("retry-on-reconnect", {"run": True})["duplicate"])
        finally:
            await server.close()
            await client.close()

    async def test_slow_history_does_not_block_command_received_or_completed(self):
        client, server = await loopback_pair()
        channels = LogicalChannels(server)
        history_ready = asyncio.Event()
        completion_ready = asyncio.Event()
        session = Session()
        try:
            history = channels.schedule("history", {"chunk": 1}, history_ready)
            await client.send(dict(command_id="run", payload={"run": True}))
            command = await server.receive()
            received = session.receive_command(**command)
            channels.schedule("command", received)
            first = await asyncio.wait_for(client.receive(), 2)
            self.assertEqual(first["channel"], "command")
            self.assertEqual(first["payload"]["ack"], "received")
            self.assertFalse(history_ready.is_set())
            self.assertFalse(history.done())
            self.assertEqual(session.commands["run"]["ack"], "received")
            # Work completion is a distinct transition and a distinct frame.
            completion_ready.set()
            channels.schedule("command", session.complete_command("run"), completion_ready)
            second = await asyncio.wait_for(client.receive(), 2)
            self.assertEqual(second["payload"]["ack"], "completed")
            self.assertFalse(history.done())
            history_ready.set()
            third = await asyncio.wait_for(client.receive(), 2)
            self.assertEqual(third["channel"], "history")
            await history
        finally:
            await channels.close()
            await client.close()

    async def test_completed_channel_tasks_release_capacity(self):
        client, server = await loopback_pair()
        channels = LogicalChannels(server, pending_limit=1)
        try:
            for index in range(3):
                task = channels.schedule("command", {"index": index})
                self.assertEqual((await client.receive())["payload"]["index"], index)
                await task
                await asyncio.sleep(0)  # allow done callback to reap the slot
                self.assertFalse(channels.tasks)
        finally:
            await channels.close()
            await client.close()

    async def test_channel_failure_is_reported_on_shutdown(self):
        client, server = await loopback_pair()
        channels = LogicalChannels(server)
        task = channels.schedule("history", {"too_big": "x" * MAX_FRAME})
        with self.assertRaises(ProtocolError):
            await task
        with self.assertRaises(ProtocolError):
            await channels.close()
        self.assertIsNone(await client.receive())
        await client.close()

    async def test_stream_shutdown_cancels_unready_history_and_delivers_eof(self):
        client, server = await loopback_pair()
        channels = LogicalChannels(server, pending_limit=1)
        pending = channels.schedule("history", {}, asyncio.Event())
        with self.assertRaisesRegex(ProtocolError, "capacity"):
            channels.schedule("history", {})
        await channels.close()
        self.assertTrue(pending.cancelled())
        self.assertIsNone(await asyncio.wait_for(client.receive(), 2))
        with self.assertRaisesRegex(ProtocolError, "closed"):
            channels.schedule("command", {})
        await client.close()

    async def test_benchmark_measures_causality_not_speed_threshold(self):
        report = await asyncio.wait_for(run_benchmark(events=4, interval=0.003,
                                                     poll_interval=0.008, delay=0.001), 5)
        self.assertIn("ARTIFICIAL", report["experiment"])
        self.assertEqual(report["inputs"]["events"], 4)
        polling, stream = report["results"]
        self.assertEqual(stream["requests"], 1)
        self.assertEqual(polling["frames"], polling["requests"] * 2)
        self.assertEqual(stream["frames"], 5)
        for result in (polling, stream):
            self.assertEqual(len(result["samples_ms"]), 4)
            self.assertTrue(all(value >= 0 for value in result["samples_ms"]))
            self.assertGreater(result["application_bytes"], 0)
            self.assertIn("in order", result["causal_assertions"])
            trace = result["causal_trace_ms"]
            ready = {item["sequence"]: item["at"] for item in trace if item["kind"] == "ready"}
            observed = [item for item in trace if item["kind"] == "observed"]
            self.assertEqual([item["sequence"] for item in observed], [1, 2, 3, 4])
            self.assertTrue(all(item["at"] >= ready[item["sequence"]] for item in observed))
        # Deliberately no stream-faster or latency-ratio assertion.
        json.dumps(report, allow_nan=False)

    async def test_benchmark_rejects_invalid_inputs(self):
        for options in (dict(events=0), dict(events=65), dict(delay=-1),
                        dict(interval=0), dict(poll_interval=float("nan")),
                        dict(delay=float("inf"))):
            with self.subTest(options=options), self.assertRaises(ValueError):
                await run_benchmark(**options)

    async def test_zero_delay_and_events_faster_than_transport(self):
        for delay in (0, 0.002):
            with self.subTest(delay=delay):
                report = await asyncio.wait_for(
                    run_benchmark(events=8, interval=0.0001,
                                  poll_interval=0.002, delay=delay), 5)
                for result in report["results"]:
                    self.assertEqual(result["events"], 8)
                    self.assertEqual(len(result["samples_ms"]), 8)
                    self.assertTrue(all(value >= 0 for value in result["samples_ms"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)

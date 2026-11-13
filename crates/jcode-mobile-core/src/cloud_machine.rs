//! Read-only, point-in-time observations of the account's pinned cloud host.
//! Presence is not proof that credentials work or that files persist across rebuilds.

use crate::cloud;
use serde::Deserialize;

pub(crate) const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_NAME_BYTES: usize = 255;
const MAX_HOME_BYTES: usize = 4096;
const INVALID: &str = "Machine inspection returned an invalid response";

/// Point-in-time resource capacity, never file contents or process details.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineResources {
    pub cpu_count: u32,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
}

impl MachineResources {
    fn valid(&self) -> bool {
        (1..=65536).contains(&self.cpu_count)
            && self.memory_total_bytes > 0
            && self.memory_available_bytes <= self.memory_total_bytes
            && self.disk_total_bytes > 0
            && self.disk_available_bytes <= self.disk_total_bytes
    }
}

/// Only resource and presence/status metadata, never credential or key contents.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineSnapshot {
    #[serde(default)]
    pub resources: Option<MachineResources>,
    pub home: String,
    pub entries: Vec<String>,
    pub github_cli: bool,
    pub github_authenticated: bool,
    pub github_config_present: bool,
    /// Conventional private-key filenames in ~/.ssh (id_* without .pub, or *.pem).
    /// This is a filename heuristic, not validation of private-key contents.
    pub ssh_key_count: usize,
    /// Conventional plaintext git credential file presence, not helper/keychain status.
    pub git_credentials_present: bool,
}

// Fixed command only. No interpolation of account, path, or user input. Python
// reads directory metadata and Linux resource counters only. gh diagnostics go directly to /dev/null and
// status is the only authentication information returned. No writes or login.
pub(crate) const INSPECTION_COMMAND: &str = r#"PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; export PATH
python3 -I -B - <<'JCODE_MACHINE_INSPECT'
import os, json, shutil, subprocess, itertools
home = os.path.expanduser('~')
resources = None
try:
    with open('/proc/meminfo', encoding='ascii') as source:
        memory = {}
        for line in source:
            key, value = line.split(':', 1)
            if key in ('MemTotal', 'MemAvailable'):
                amount, unit = value.split()
                if unit != 'kB':
                    raise ValueError('unexpected memory unit')
                memory[key] = int(amount) * 1024
    cpu = os.cpu_count()
    disk = shutil.disk_usage(home)
    total, available = memory['MemTotal'], memory['MemAvailable']
    if cpu is not None and 1 <= cpu <= 65536 and 0 < total <= 2**64 - 1 and 0 <= available <= total and 0 < disk.total <= 2**64 - 1 and 0 <= disk.free <= disk.total:
        resources = dict(cpu_count=cpu, memory_total_bytes=total, memory_available_bytes=available, disk_total_bytes=disk.total, disk_available_bytes=disk.free)
except (OSError, ValueError, KeyError, OverflowError):
    pass
def safe(s, limit):
    return bool(s) and len(s.encode('utf-8')) <= limit and not any(ord(c) < 32 or ord(c) == 127 or 0xD800 <= ord(c) <= 0xDFFF for c in s)
def names(path, limit):
    try:
        with os.scandir(path) as items:
            return [e.name for e in itertools.islice(items, limit)]
    except OSError:
        return []
entries = sorted(n for n in names(home, 256) if safe(n, 255))[:64]
config = os.environ.get('XDG_CONFIG_HOME') or os.path.join(home, '.config')
gh_config = os.environ.get('GH_CONFIG_DIR') or os.path.join(config, 'gh')
gh = shutil.which('gh') is not None
auth = False
if gh:
    try:
        env = dict(os.environ, GH_PROMPT_DISABLED='1', GIT_TERMINAL_PROMPT='0', GH_NO_UPDATE_NOTIFIER='1')
        auth = subprocess.run(['gh', 'auth', 'status', '--hostname', 'github.com'], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3, env=env).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        pass
keys = names(os.path.join(home, '.ssh'), 256)
count = sum(1 for n in keys if ((n.startswith('id_') and not n.endswith('.pub')) or n.endswith('.pem')) and os.path.isfile(os.path.join(home, '.ssh', n)))
if not safe(home, 4096) or not home.startswith('/'):
    raise SystemExit(1)
print('JCODE_MACHINE_V1')
print(json.dumps(dict(resources=resources, home=home, entries=entries, github_cli=gh, github_authenticated=auth, github_config_present=os.path.isfile(os.path.join(gh_config, 'hosts.yml')), ssh_key_count=count, git_credentials_present=any(os.path.isfile(p) for p in [os.path.join(home, '.git-credentials'), os.path.join(config, 'git', 'credentials')]))))
JCODE_MACHINE_INSPECT"#;

/// Wake/reach the account's host and inspect it over pinned SSH. This may register
/// an ephemeral connection key with the control plane, but never installs tools,
/// authenticates GitHub, modifies remote files, or starts a Jcode API process.
/// The remote command has an eight-second budget and a 64 KiB stdout cap. Cloud
/// provisioning and SSH establishment use their existing separate deadlines.
pub fn inspect(account: &cloud::Account) -> Result<MachineSnapshot, String> {
    let ssh = cloud::inspection_ssh(account)?;
    parse(&ssh.inspect_machine()?)
}

fn safe_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn parse(bytes: &[u8]) -> Result<MachineSnapshot, String> {
    let invalid = || INVALID.to_string();
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err(invalid());
    }
    let bytes = bytes
        .strip_prefix(b"JCODE_MACHINE_V1\n")
        .ok_or_else(invalid)?;
    let snapshot: MachineSnapshot = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if !safe_text(&snapshot.home, MAX_HOME_BYTES)
        || !snapshot.home.starts_with('/')
        || snapshot.entries.len() > MAX_ENTRIES
        || snapshot.entries.iter().any(|name| {
            !safe_text(name, MAX_NAME_BYTES) || name.contains('/') || name == "." || name == ".."
        })
        || snapshot.ssh_key_count > 256
        || (snapshot.github_authenticated && !snapshot.github_cli)
        || snapshot
            .resources
            .as_ref()
            .is_some_and(|resources| !resources.valid())
    {
        return Err(invalid());
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> serde_json::Value {
        json!({"home":"/home/person", "entries":[".ssh", "project with spaces"], "github_cli":true,
            "github_authenticated":false, "github_config_present":true, "ssh_key_count":2,
            "git_credentials_present":false})
    }
    fn response(value: serde_json::Value) -> Vec<u8> {
        format!("JCODE_MACHINE_V1\n{value}\n").into_bytes()
    }
    #[test]
    fn parses_only_metadata() {
        let result = parse(&response(valid())).unwrap();
        assert_eq!(result.home, "/home/person");
        assert_eq!(result.entries, [".ssh", "project with spaces"]);
        assert!(!result.github_authenticated);
        assert_eq!(result.ssh_key_count, 2);
        assert_eq!(result.resources, None);
    }
    fn resources() -> serde_json::Value {
        json!({"cpu_count":8, "memory_total_bytes":8192, "memory_available_bytes":4096,
            "disk_total_bytes":16384, "disk_available_bytes":8192})
    }
    #[test]
    fn parses_optional_resources_and_boundary_values() {
        let mut v = valid();
        v["resources"] = serde_json::Value::Null;
        assert_eq!(parse(&response(v.clone())).unwrap().resources, None);
        for cpu in [1, 65536] {
            let mut r = resources();
            r["cpu_count"] = json!(cpu);
            r["memory_available_bytes"] = r["memory_total_bytes"].clone();
            r["disk_available_bytes"] = json!(0);
            v["resources"] = r;
            let parsed = parse(&response(v.clone())).unwrap().resources.unwrap();
            assert_eq!(parsed.cpu_count, cpu);
            assert_eq!(parsed.memory_available_bytes, parsed.memory_total_bytes);
            assert_eq!(parsed.disk_available_bytes, 0);
        }
    }
    #[test]
    fn rejects_invalid_resources() {
        for (key, value) in [
            ("cpu_count", json!(0)),
            ("cpu_count", json!(65537)),
            ("cpu_count", json!(-1)),
            ("memory_total_bytes", json!(0)),
            ("memory_available_bytes", json!(8193)),
            ("memory_available_bytes", json!(-1)),
            ("disk_total_bytes", json!(0)),
            ("disk_available_bytes", json!(16385)),
            ("disk_available_bytes", json!(-1)),
            ("extra", json!(1)),
        ] {
            let mut v = valid();
            let mut r = resources();
            r[key] = value;
            v["resources"] = r;
            assert_eq!(parse(&response(v)).unwrap_err(), INVALID, "{key}");
        }
        let mut v = valid();
        v["resources"] = json!({"cpu_count":1});
        assert_eq!(parse(&response(v)).unwrap_err(), INVALID);
    }
    #[test]
    fn rejects_unframed_trailing_and_secret_output_without_echo() {
        for bytes in [
            b"auth secret-token".to_vec(),
            [response(valid()), b"secret-token".to_vec()].concat(),
            vec![b'x'; MAX_OUTPUT_BYTES + 1],
        ] {
            assert_eq!(parse(&bytes).unwrap_err(), INVALID);
        }
        let mut v = valid();
        v["secret"] = json!("secret-token");
        assert_eq!(parse(&response(v)).unwrap_err(), INVALID);
    }
    #[test]
    fn bounds_and_validates_every_returned_name() {
        for name in [
            "../private",
            "/etc/passwd",
            "..",
            ".",
            "bad\nname",
            "bad\u{1b}name",
            "",
        ] {
            let mut v = valid();
            v["entries"] = json!([name]);
            assert!(parse(&response(v)).is_err());
        }
        for (key, value) in [
            ("home", json!("relative")),
            ("home", json!("/x\nsecret")),
            ("entries", json!(vec!["a"; 65])),
            ("entries", json!(["x".repeat(256)])),
            ("ssh_key_count", json!(257)),
            ("ssh_key_count", json!(-1)),
            ("github_cli", json!(false)),
        ] {
            let mut v = valid();
            v[key] = value;
            if key == "github_cli" {
                v["github_authenticated"] = json!(true);
            }
            assert!(parse(&response(v)).is_err(), "{key}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn unavailable_resource_stats_do_not_fail_inspection() {
        let command = INSPECTION_COMMAND.replace("'/proc/meminfo'", "'/dev/null/jcode-meminfo'");
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .env("GH_PROMPT_DISABLED", "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(parse(&output.stdout).unwrap().resources, None);
    }

    #[cfg(unix)]
    #[test]
    fn fixed_command_observes_fixture_metadata_without_secret_contents() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("jcode-inspection-{}", uuid::Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        for directory in [".ssh", ".config/gh", ".local/bin", "project"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        for name in [
            ".ssh/id_ed25519",
            ".ssh/id_ed25519.pub",
            ".ssh/config",
            ".git-credentials",
            ".config/gh/hosts.yml",
        ] {
            std::fs::write(root.join(name), "NEVER_RETURN_SECRET_CONTENTS").unwrap();
        }
        let gh = root.join(".local/bin/gh");
        std::fs::write(
            &gh,
            "#!/bin/sh\n[ \"$*\" = \"auth status --hostname github.com\" ] || exit 1\necho NEVER_RETURN_AUTH_DIAGNOSTICS >&2\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new("sh")
            .args(["-c", INSPECTION_COMMAND])
            .env_clear()
            .env("HOME", &root)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(output.status.success(), "inspection command failed");
        assert!(output.stderr.is_empty());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("NEVER_RETURN"));
        let snapshot = parse(&output.stdout).unwrap();
        #[cfg(target_os = "linux")]
        assert!(snapshot.resources.as_ref().unwrap().valid());
        assert_eq!(snapshot.home, root.to_str().unwrap());
        assert!(
            snapshot.github_cli && snapshot.github_authenticated && snapshot.github_config_present
        );
        assert!(snapshot.git_credentials_present);
        assert_eq!(snapshot.ssh_key_count, 1);
        assert!(snapshot.entries.contains(&"project".to_string()));
        assert_eq!(
            std::fs::read_to_string(root.join(".ssh/id_ed25519")).unwrap(),
            "NEVER_RETURN_SECRET_CONTENTS"
        );
    }
}

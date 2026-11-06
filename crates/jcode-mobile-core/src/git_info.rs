//! Bounded, read-only Git metadata over the account's pinned inspection SSH.
use crate::cloud;
use serde::Deserialize;

const INVALID: &str = "Git inspection failed or returned an invalid response";
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSnapshot {
    pub root: String,
    pub branch: String,
    pub status: String,
    pub recent_commits: String,
}

/// Inspect an existing repository, without fetching, writing the index, running
/// hooks or starting the API server. Detached and unborn HEADs are supported.
/// SSH execution is bounded to eight seconds and 64 KiB, including stderr.
/// Provisioning and pinned SSH authentication retain their separate deadlines.
pub fn inspect(account: &cloud::Account, working_dir: &str) -> Result<GitSnapshot, String> {
    inspection_command(working_dir)?;
    let ssh = cloud::inspection_ssh(account)?;
    parse(
        &ssh.inspect_git(working_dir)
            .map_err(|_| INVALID.to_string())?,
    )
}

pub(crate) fn inspection_command(working_dir: &str) -> Result<String, String> {
    if working_dir.is_empty()
        || working_dir.len() > 4096
        || working_dir.chars().any(char::is_control)
    {
        return Err("Invalid Git working directory".into());
    }
    let quoted = format!("'{}'", working_dir.replace('\'', "'\"'\"'"));
    Ok(format!(
        "python3 -I -B - {quoted} <<'JCODE_GIT_INSPECT'\n{SCRIPT}\nJCODE_GIT_INSPECT"
    ))
}

// No remote/config listing, patch contents, credential helpers, network commands,
// external diff, hooks or pager. Pipe reads and process lifetime are both bounded.
const SCRIPT: &str = r#"import os, sys, json, subprocess, selectors, time, re
end = time.monotonic() + 6
budget = 48000
env = {k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
env.update(GIT_OPTIONAL_LOCKS='0', GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null', LC_ALL='C')
def git(args, optional=False):
    global budget
    p = subprocess.Popen(['git', '--no-pager', '--no-optional-locks', '-c', 'core.fsmonitor=false', '-c', 'core.hooksPath=/dev/null', '-C', sys.argv[1]] + args, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        out = bytearray()
        with selectors.DefaultSelector() as sel:
            sel.register(p.stdout, selectors.EVENT_READ)
            while sel.get_map():
                remaining = end - time.monotonic()
                if remaining <= 0: raise RuntimeError()
                for key, event in sel.select(remaining):
                    data = os.read(key.fileobj.fileno(), 4096)
                    budget -= len(data)
                    if budget < 0: raise RuntimeError()
                    if data: out.extend(data)
                    else: sel.unregister(key.fileobj)
        rc = p.wait(timeout=max(0.001, end-time.monotonic()))
        if rc and not optional: raise RuntimeError()
        return None if rc else out.decode('utf-8', 'replace').rstrip('\n')
    finally:
        if p.poll() is None: p.kill()
        p.wait()
        p.stdout.close()
def clean(s):
    # Defense in depth for credentials embedded in a filename or commit subject.
    s = re.sub(r'([a-zA-Z][a-zA-Z0-9+.-]*://)[^/\s]*@', r'\1[redacted]@', s)
    return ''.join(c if c in '\n\t' or ord(c) >= 32 and ord(c) != 127 else '?' for c in s)
try:
    root = git(['rev-parse', '--show-toplevel'])
    branch = git(['symbolic-ref', '--quiet', '--short', 'HEAD'], True) or '(detached HEAD)'
    status = git(['status', '--porcelain=v1', '--untracked-files=normal', '--ignore-submodules=all'])
    head = git(['rev-parse', '--verify', 'HEAD'], True)
    commits = git(['log', '-10', '--no-show-signature', '--format=%h %s', '--no-decorate', 'HEAD', '--']) if head else ''
    print('JCODE_GIT_V1')
    print(json.dumps(dict(root=clean(root), branch=clean(branch), status=clean(status), recent_commits=clean(commits))))
except Exception:
    raise SystemExit(1)
"#;

fn parse(bytes: &[u8]) -> Result<GitSnapshot, String> {
    let invalid = || INVALID.to_string();
    if bytes.len() > MAX_OUTPUT {
        return Err(invalid());
    }
    let json = bytes.strip_prefix(b"JCODE_GIT_V1\n").ok_or_else(invalid)?;
    let snapshot: GitSnapshot = serde_json::from_slice(json).map_err(|_| invalid())?;
    if !snapshot.root.starts_with('/')
        || snapshot.root.len() > 4096
        || snapshot.branch.is_empty()
        || snapshot.branch.len() > 4096
        || [
            &snapshot.root,
            &snapshot.branch,
            &snapshot.status,
            &snapshot.recent_commits,
        ]
        .iter()
        .any(|s| s.chars().any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        return Err(invalid());
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn rejects_invalid_input_and_untrusted_responses() {
        for path in ["", "x\nwhoami", "x\0", &"x".repeat(4097)] {
            assert!(inspection_command(path).is_err());
        }
        for bytes in [b"secret-token".to_vec(), vec![b'x'; MAX_OUTPUT + 1],
            b"JCODE_GIT_V1\n{\"root\":\"/x\",\"branch\":\"main\",\"status\":\"\",\"recent_commits\":\"\",\"secret\":true}".to_vec()] {
            assert_eq!(parse(&bytes).unwrap_err(), INVALID);
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_repository_is_read_only_and_shell_quoted() {
        let root = std::env::temp_dir().join(format!(
            "jcode-git-' ; $(touch INJECTED)-{}",
            uuid::Uuid::new_v4()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git fixture failed");
        };
        git(&["init", "-b", "main"]);
        let inspect_local = || {
            let output = Command::new("sh")
                .args(["-c", &inspection_command(root.to_str().unwrap()).unwrap()])
                .output()
                .unwrap();
            assert!(output.status.success(), "inspection failed");
            assert!(output.stderr.is_empty());
            parse(&output.stdout).unwrap()
        };
        let unborn = inspect_local();
        assert_eq!(unborn.branch, "main");
        assert!(unborn.recent_commits.is_empty());
        std::fs::write(root.join("tracked"), "original").unwrap();
        git(&["add", "tracked"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "first https://user:secret@example.com/repo",
        ]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://user:REMOTE_SECRET@example.com/repo",
        ]);
        let index = std::fs::read(root.join(".git/index")).unwrap();
        std::fs::write(root.join("tracked"), "modified").unwrap();
        std::fs::write(root.join("untracked"), "private content").unwrap();
        let snapshot = inspect_local();
        assert_eq!(snapshot.root, root.to_str().unwrap());
        assert!(snapshot.status.contains(" M tracked"));
        assert!(snapshot.status.contains("?? untracked"));
        assert!(
            snapshot
                .recent_commits
                .contains("first https://[redacted]@example.com/repo")
        );
        assert!(!format!("{snapshot:?}").contains("secret"));
        assert!(!format!("{snapshot:?}").contains("REMOTE_SECRET"));
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert!(!root.join(".git/index.lock").exists());
        git(&["checkout", "--detach"]);
        assert_eq!(inspect_local().branch, "(detached HEAD)");
        let output = Command::new("sh")
            .args([
                "-c",
                &inspection_command(root.join("missing").to_str().unwrap()).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            &"x".repeat(50000),
        ]);
        let output = Command::new("sh")
            .args(["-c", &inspection_command(root.to_str().unwrap()).unwrap()])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn remote_script_kills_a_stalled_git_process() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("jcode-git-timeout-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let fake = root.join("git");
        std::fs::write(&fake, "#!/usr/bin/python3\nimport time\ntime.sleep(60)\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = std::time::Instant::now();
        let output = Command::new("/bin/sh")
            .args(["-c", &inspection_command("/unused").unwrap()])
            .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
            .output()
            .unwrap();
        let elapsed = start.elapsed();
        std::fs::remove_dir_all(root).unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        assert!(elapsed < std::time::Duration::from_secs(9));
    }
}

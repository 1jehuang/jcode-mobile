//! Bounded, read-only Git metadata over the account's pinned inspection SSH.
use crate::cloud;
use serde::Deserialize;

const INVALID: &str = "Git inspection failed or returned an invalid response";
/// Returned when the directory exists but is not inside a Git work tree.
pub const NOT_A_REPOSITORY: &str = "This directory is not inside a Git repository";
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSnapshot {
    pub root: String,
    pub branch: String,
    pub status: String,
    /// Up to ten newline-separated `short hash\tsubject\trelative age` records.
    pub recent_commits: String,
    /// Per-file additions/deletions relative to HEAD, including staged changes.
    #[serde(default)]
    pub diff_stat: String,
    /// Local commits not in the configured upstream, or zero without an upstream.
    #[serde(default)]
    pub ahead: u32,
    /// Upstream commits not in HEAD, or zero without an upstream.
    #[serde(default)]
    pub behind: u32,
}

/// Inspect an existing repository, without fetching, writing the index, running
/// hooks or starting the API server. Detached and unborn HEADs are supported.
/// SSH execution is bounded to eight seconds and 64 KiB, including stderr.
/// Provisioning and pinned SSH authentication retain their separate deadlines.
pub fn inspect(account: &cloud::Account, working_dir: &str) -> Result<GitSnapshot, String> {
    inspection_command(working_dir)?;
    let ssh = cloud::inspection_ssh(account)?;
    let output = match ssh.inspect_git(working_dir) {
        Ok(output) => output,
        // A reused master may have died while the app was suspended.
        // Reconnect once instead of reporting a stale socket as a Git failure.
        Err(_) if ssh.is_closed() => {
            cloud::forget_inspection_ssh(&ssh);
            cloud::inspection_ssh(account)?
                .inspect_git(working_dir)
                .map_err(|_| INVALID.to_string())?
        }
        Err(_) => return Err(INVALID.to_string()),
    };
    parse(&output)
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
sys.argv[1] = os.path.expanduser(sys.argv[1])
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
    root = git(['rev-parse', '--show-toplevel'], True)
    if root is None:
        if not os.path.isdir(sys.argv[1]): raise RuntimeError()
        print('JCODE_GIT_NOT_REPO')
        raise SystemExit(0)
    branch = git(['symbolic-ref', '--quiet', '--short', 'HEAD'], True) or '(detached HEAD)'
    status = git(['status', '--porcelain=v1', '--untracked-files=normal', '--ignore-submodules=all'])
    head = git(['rev-parse', '--verify', 'HEAD'], True)
    commits = git(['log', '-10', '--no-show-signature', '--format=%h%x09%s%x09%ar', '--no-decorate', 'HEAD', '--']) if head else ''
    diff_args = ['diff', '--no-ext-diff', '--no-textconv', '--ignore-submodules=all', '--numstat']
    diff_stat = git(diff_args + (['HEAD'] if head else ['--cached']) + ['--'])
    counts = git(['rev-list', '--left-right', '--count', 'HEAD...@{upstream}', '--'], True) if head else None
    ahead, behind = map(int, counts.split()) if counts else (0, 0)
    print('JCODE_GIT_V1')
    print(json.dumps(dict(root=clean(root), branch=clean(branch), status=clean(status), recent_commits=clean(commits), diff_stat=clean(diff_stat), ahead=ahead, behind=behind)))
except Exception:
    raise SystemExit(1)
"#;

fn parse(bytes: &[u8]) -> Result<GitSnapshot, String> {
    let invalid = || INVALID.to_string();
    if bytes.len() > MAX_OUTPUT {
        return Err(invalid());
    }
    if bytes.trim_ascii() == b"JCODE_GIT_NOT_REPO" {
        return Err(NOT_A_REPOSITORY.to_string());
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
            &snapshot.diff_stat,
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
    fn legacy_snapshot_defaults_new_metadata() {
        let snapshot = parse(b"JCODE_GIT_V1\n{\"root\":\"/x\",\"branch\":\"main\",\"status\":\"\",\"recent_commits\":\"abc subject\"}").unwrap();
        assert!(snapshot.diff_stat.is_empty());
        assert_eq!((snapshot.ahead, snapshot.behind), (0, 0));
    }

    #[test]
    fn non_repository_is_reported_distinctly() {
        assert_eq!(parse(b"JCODE_GIT_NOT_REPO\n").unwrap_err(), NOT_A_REPOSITORY);
    }

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
        assert!(unborn.diff_stat.is_empty());
        assert_eq!((unborn.ahead, unborn.behind), (0, 0));
        std::fs::write(root.join("tracked"), "original").unwrap();
        git(&["add", "tracked"]);
        assert_eq!(inspect_local().diff_stat, "1\t0\ttracked");
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
        git(&["branch", "upstream"]);
        git(&["branch", "--set-upstream-to=upstream"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "local",
        ]);
        assert_eq!((inspect_local().ahead, inspect_local().behind), (1, 0));
        git(&["checkout", "upstream"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "upstream",
        ]);
        git(&["checkout", "main"]);
        assert_eq!((inspect_local().ahead, inspect_local().behind), (1, 1));
        std::fs::write(root.join("staged"), "staged\n").unwrap();
        git(&["add", "staged"]);
        let index = std::fs::read(root.join(".git/index")).unwrap();
        std::fs::write(root.join("tracked"), "modified").unwrap();
        std::fs::write(root.join("untracked"), "private content").unwrap();
        let snapshot = inspect_local();
        assert_eq!(snapshot.root, root.to_str().unwrap());
        assert!(snapshot.status.contains(" M tracked"));
        assert!(snapshot.status.contains("?? untracked"));
        assert!(snapshot.diff_stat.contains("1\t0\tstaged"));
        assert!(snapshot.diff_stat.contains("1\t1\ttracked"));
        assert!(!snapshot.diff_stat.contains("untracked"));
        for commit in snapshot.recent_commits.lines() {
            let columns: Vec<_> = commit.split('\t').collect();
            assert_eq!(columns.len(), 3);
            assert!(!columns[2].is_empty());
        }
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
        let plain = std::env::temp_dir().join(format!("jcode-git-plain-{}", uuid::Uuid::new_v4()));
        let _plain_cleanup = Cleanup(plain.clone());
        std::fs::create_dir_all(&plain).unwrap();
        let output = Command::new("sh")
            .args(["-c", &inspection_command(plain.to_str().unwrap()).unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(parse(&output.stdout).unwrap_err(), NOT_A_REPOSITORY);
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

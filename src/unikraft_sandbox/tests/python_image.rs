//! End-to-end tests against the published Python rootfs images.
//!
//! They need KVM (or MSHV / WHP) and the images the `hluk` CLI caches
//! (`hluk run --runtime python`, `hluk run --runtime agent`); without a
//! usable hypervisor or an image a test says so and passes, as there is
//! nothing to run it on.

use hyperlight_sandbox::{Sandbox, SandboxBuilder, ToolRegistry};
use hyperlight_unikraft_sandbox::{Unikraft, cached_rootfs};

fn rootfs(runtime: &str) -> Option<String> {
    if !hyperlight_unikraft::hyperlight_host::is_hypervisor_present() {
        eprintln!("skipping: no usable hypervisor");
        return None;
    }
    match cached_rootfs(runtime) {
        Ok(path) => Some(path.display().to_string()),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn python_sandbox(
    tools: ToolRegistry,
    input: Option<&std::path::Path>,
) -> Option<Sandbox<Unikraft>> {
    let rootfs = rootfs("python")?;
    let mut builder = SandboxBuilder::new()
        .module_path(rootfs)
        .with_tools(tools)
        .temp_output();
    if let Some(input) = input {
        builder = builder.input_dir(input);
    }
    Some(builder.guest(Unikraft::new()).build().expect("boot"))
}

#[test]
fn run_semantics_match_the_other_backends() {
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("echo", Ok);
    // A reply over the 64 KiB host call limit takes the blob path.
    tools.register_unvalidated("big", |_| Ok(serde_json::json!("x".repeat(300_000))));
    let input = tempfile::tempdir().unwrap();
    std::fs::write(input.path().join("in.txt"), "from the host").unwrap();
    let Some(mut sandbox) = python_sandbox(tools, Some(input.path())) else {
        return;
    };

    // stdout and stderr are separate, and an exception is exit code 1.
    let r = sandbox
        .run("import sys\nprint('out')\nprint('err', file=sys.stderr)\nraise ValueError('boom')")
        .unwrap();
    assert_eq!(r.stdout, "out\n");
    assert!(r.stderr.starts_with("err\n"), "{:?}", r.stderr);
    assert!(r.stderr.contains("ValueError: boom"), "{:?}", r.stderr);
    assert_eq!(r.exit_code, 1);

    // sys.exit carries its status; sys.exit() is success.
    assert_eq!(sandbox.run("import sys; sys.exit(3)").unwrap().exit_code, 3);
    assert_eq!(sandbox.run("import sys; sys.exit()").unwrap().exit_code, 0);

    // State persists from one run to the next.
    sandbox.run("x = 20").unwrap();
    assert_eq!(sandbox.run("print(x + 1)").unwrap().stdout, "21\n");

    // Tools, small and large, and their errors.
    let r = sandbox
        .run(
            "print(call_tool('echo', a=1, s='hi'))\n\
             print(len(call_tool('big')))\n\
             try:\n    call_tool('missing')\nexcept RuntimeError as e:\n    print('error:', e)",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines[0], "{'a': 1, 's': 'hi'}");
    assert_eq!(lines[1], "300000");
    assert!(
        lines[2].starts_with("error: Tool 'missing' failed"),
        "{}",
        lines[2]
    );

    // Code and output over the host call limit.
    let code = format!(
        "s = {:?}\nprint(len(s))\nprint('y' * 200000)",
        "z".repeat(100_000)
    );
    let r = sandbox.run(&code).unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert!(r.stdout.starts_with("100000\n"));
    assert_eq!(r.stdout.len(), "100000\n".len() + 200_001);

    // /input is read-only, /output is writable and kept between runs.
    let r = sandbox
        .run(
            "print(open('/input/in.txt').read())\n\
             open('/output/out.txt', 'w').write('done')\n\
             try:\n    open('/input/new.txt', 'w')\nexcept OSError as e:\n    print('ro:', e.errno)",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert!(r.stdout.starts_with("from the host\nro: "), "{}", r.stdout);
    assert_eq!(
        sandbox.get_output_files().unwrap(),
        vec!["out.txt".to_string()]
    );
    let r = sandbox
        .run("print(open('/output/out.txt').read())")
        .unwrap();
    assert_eq!(r.stdout, "done\n", "{}", r.stderr);

    // Network is off unless allowed, and the denial is the host's.
    let r = sandbox
        .run("try:\n    http_get('https://example.com/')\nexcept OSError as e:\n    print(e)")
        .unwrap();
    assert!(r.stdout.contains("denied"), "{}", r.stdout);

    // Snapshot / restore rewinds the interpreter, and clears /output.
    let snapshot = sandbox.snapshot().unwrap();
    sandbox.run("x = 99").unwrap();
    sandbox.restore(&snapshot).unwrap();
    assert_eq!(sandbox.run("print(x)").unwrap().stdout, "20\n");
    assert!(sandbox.get_output_files().unwrap().is_empty());
}

#[test]
fn python_and_native_output_keep_their_order() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let r = sandbox
        .run("import os\nprint('before', flush=True)\nos.write(1, b'middle\\n')\nprint('after')")
        .unwrap();
    assert_eq!(r.stdout, "before\nmiddle\nafter\n", "{}", r.stderr);
    // No clean split when standard error joins a partial standard output
    // line: standard output keeps both, and standard error its own.
    let r = sandbox
        .run("import os\nos.write(1, b'out ')\nos.write(2, b'err\\n')")
        .unwrap();
    assert_eq!(r.stdout, "out err\n");
    assert_eq!(r.stderr, "err\n");
    // Closing standard output, or fd 1, costs the run nothing after it.
    let r = sandbox
        .run("import os, sys\nprint('gone')\nsys.stdout.close()")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let r = sandbox.run("import os\nos.close(1)").unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let r = sandbox
        .run("import sys\nprint('back')\nprint('err', file=sys.stderr)")
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_str(), r.stderr.as_str()),
        (0, "back\n", "err\n")
    );
    // Nor does detaching it, or closing its descriptor; and a file that
    // takes the descriptor's number gets none of the output.
    let r = sandbox
        .run("import io, sys\nsys.stdout = io.TextIOWrapper(sys.stdout.detach(), encoding='utf-8')\nprint('detached')")
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_str()),
        (0, "detached\n"),
        "{}",
        r.stderr
    );
    let r = sandbox
        .run("import os, sys\nos.close(sys.stdout.fileno())\nf = open('/tmp/taken', 'w')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let r = sandbox
        .run("print('still here')\nf.close()\nprint(repr(open('/tmp/taken').read()))")
        .unwrap();
    assert_eq!(r.stdout, "still here\n''\n", "{}", r.stderr);
    // Nor does a file in /output, whose inode the console's may share.
    let r = sandbox
        .run("import os, sys\nopen('/output/a', 'w').close()\nos.close(sys.stdout.fileno())\ng = open('/output/b', 'w')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let r = sandbox
        .run("print('kept')\ng.close()\nprint(repr(open('/output/b').read()))")
        .unwrap();
    assert_eq!(r.stdout, "kept\n''\n", "{}", r.stderr);
    // What holds on to standard output keeps it from run to run.
    sandbox
        .run("import logging, sys\nlog = logging.getLogger('t')\nlog.addHandler(logging.StreamHandler(sys.stdout))")
        .unwrap();
    let r = sandbox.run("log.warning('logged')").unwrap();
    assert_eq!(r.stdout, "logged\n", "{}", r.stderr);
}

#[test]
fn guest_classes_pickle_as_in_a_python_process() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    sandbox
        .run("import pickle\nclass Point:\n    def __init__(self, x):\n        self.x = x\ndata = pickle.dumps(Point(3))")
        .unwrap();
    let r = sandbox
        .run("print(pickle.loads(data).x, __name__)\nif __name__ == '__main__':\n    print('main')")
        .unwrap();
    assert_eq!(r.stdout, "3 __main__\nmain\n", "{}", r.stderr);
}

/// The guest's file calls are refused the write that would break the
/// filesystem limits, as the other backends refuse it, before anything
/// reaches the host directory.
#[test]
fn output_limits_refuse_the_write_that_breaks_them() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let limits = hyperlight_sandbox::FilesystemLimits::new(1024, 4096, 2).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .filesystem_limits(limits)
        .temp_output()
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run(
            "import errno, os\n\
             def attempt(path, size):\n    \
                 try:\n        \
                     with open(path, 'wb') as f:\n            \
                         f.write(b'0' * size)\n        \
                     return 'ok'\n    \
                 except OSError as e:\n        \
                     return errno.errorcode[e.errno]\n\
             print(attempt('/output/big.bin', 2048), os.path.getsize('/output/big.bin'))\n\
             os.remove('/output/big.bin')\n\
             print(attempt('/output/a', 1000), attempt('/output/b', 1000), attempt('/output/c', 1))",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "EFBIG 0\nok ok EDQUOT\n", "{}", r.stderr);
    let mut files = sandbox.get_output_files().unwrap();
    files.sort();
    assert_eq!(files, ["a", "b"]);
}

#[test]
fn agent_image_has_the_data_science_stack() {
    let Some(rootfs) = rootfs("agent") else {
        return;
    };
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run(
            "import numpy, pandas, scipy, sklearn\n\
             print(int(numpy.arange(10).sum()), len(pandas.DataFrame({'a': [1, 2]})))",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "45 2\n");
}

#[test]
fn agent_image_runs_child_processes_and_threads() {
    let Some(rootfs) = rootfs("agent") else {
        return;
    };
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run(
            "import subprocess, threading\n\
             def run(args, **kw):\n    return subprocess.run(args, capture_output=True, text=True, **kw).stdout.strip()\n\
             print(run(['echo', 'child']))\n\
             print(run('echo a b c | wc -w', shell=True))\n\
             out = []\n\
             t = threading.Thread(target=lambda: out.append(run(['echo', 'thread'])))\n\
             t.start(); t.join()\n\
             print(out[0])\n\
             try:\n    subprocess.run(['sleep', '30'], timeout=0.5)\nexcept subprocess.TimeoutExpired:\n    print('killed')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "child\n3\nthread\nkilled\n");
    // The kernel's log of each PATH entry a program is not found in is the
    // guest log, not the program's standard error.
    assert!(!r.stderr.contains("ERR:"), "{}", r.stderr);
}

#[test]
fn multibyte_output_and_large_requests_get_through() {
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("echo", Ok);
    let Some(mut sandbox) = python_sandbox(tools, None) else {
        return;
    };

    // An emoji is 12 bytes of JSON: output chunks must still fit a host call.
    let r = sandbox
        .run("print('😀' * 9000)\nprint('é' * 30000)\nprint('after')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        format!("{}\n{}\nafter\n", "😀".repeat(9000), "é".repeat(30000))
    );

    // Tool arguments and HTTP bodies over the host call limit are uploaded.
    let r = sandbox
        .run(
            "print(len(call_tool('echo', s='😀' * 50000)['s']))\n\
             try:\n    http_post('https://example.com/', body='x' * 200000)\n\
             except OSError as e:\n    print(e)",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines[0], "50000");
    // Refused by the allowlist, so the request reached the host whole.
    assert!(lines[1].contains("denied"), "{}", r.stdout);
}

#[test]
fn unread_replies_are_capped_per_run() {
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("mb", |_| Ok(serde_json::json!("x".repeat(1 << 20))));
    let Some(mut sandbox) = python_sandbox(tools, None) else {
        return;
    };
    // Calling the host function directly leaves each reply unread.
    let r = sandbox
        .run(
            "import hyperlight\n\
             try:\n    for i in range(200):\n        hyperlight.call('sandbox.call_tool', 'mb', {})\n\
             except hyperlight.HostError as e:\n    print(i, e)",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    // 64 MiB of pending replies at most: 63 of these (each 1 MiB and
    // its JSON quotes) fit.
    assert!(r.stdout.starts_with("63 "), "{}", r.stdout);
    assert!(r.stdout.contains("unread replies"), "{}", r.stdout);
    // The next run starts with nothing held.
    let r = sandbox.run("print(len(call_tool('mb')))").unwrap();
    assert_eq!(r.stdout, "1048576\n", "{}", r.stderr);
}

/// What the host puts in `/output` during a run, which the guest's file
/// calls could not have been refused, is checked once the run is over:
/// whatever the run's status, its output is kept and the check made.
#[test]
fn out_of_range_exit_status_keeps_output_and_limits() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    let host_side = output.path().to_owned();
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("spoil", move |_| {
        std::fs::write(host_side.join("big.bin"), vec![0u8; 2048]).unwrap();
        Ok(serde_json::Value::Null)
    });
    let limits = hyperlight_sandbox::FilesystemLimits::new(1024, 4096, 2).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .with_tools(tools)
        .filesystem_limits(limits)
        .output_dir(
            output.path(),
            hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::READ | hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run("call_tool('spoil')\nprint('before')\nimport sys\nsys.exit(2 ** 40)")
        .unwrap();
    assert_eq!(r.exit_code, 1);
    assert_eq!(r.stdout, "before\n");
    assert!(
        r.stderr.contains("exceeded the filesystem limits"),
        "{}",
        r.stderr
    );
    assert!(sandbox.get_output_files().unwrap().is_empty());
}

/// Through `CapFs`, `/input` and `/output` still work as a filesystem
/// does for the libraries a script uses. (Not `shutil.rmtree` or
/// `shutil.copytree`, which no hostfs mount supports yet: the guest gives
/// a path different inode numbers by `lstat` and `fstat`, and has no
/// `utime`.)
#[test]
fn input_and_output_work_as_a_filesystem() {
    let input = tempfile::tempdir().unwrap();
    std::fs::create_dir(input.path().join("data")).unwrap();
    std::fs::write(input.path().join("data/in.txt"), "in").unwrap();
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), Some(input.path())) else {
        return;
    };
    let r = sandbox
        .run(
            "import os, shutil, tempfile\n\
             print(open('/input/data/in.txt').read(), os.listdir('/input'))\n\
             os.makedirs('/output/a/b', exist_ok=True)\n\
             os.makedirs('/output/a/b', exist_ok=True)\n\
             with tempfile.NamedTemporaryFile('w', dir='/output/a', delete=False) as f:\n    \
                 f.write('atomic')\n\
             os.replace(f.name, '/output/a/b/result.txt')\n\
             with open('/output/log', 'a') as f:\n    \
                 f.write('one ')\n\
             with open('/output/log', 'a') as f:\n    \
                 f.write('two')\n\
             with open('/output/log', 'r+') as f:\n    \
                 f.seek(4)\n    \
                 f.write('TWO')\n    \
                 f.truncate()\n\
             os.makedirs('/output/copy/b')\n\
             shutil.copyfile('/output/a/b/result.txt', '/output/copy/b/result.txt')\n\
             for d, dirs, fs in os.walk('/output/a', topdown=False):\n    \
                 for name in fs:\n        \
                     os.remove(os.path.join(d, name))\n    \
                 os.rmdir(d)\n\
             print(sorted((d, sorted(fs)) for d, _, fs in os.walk('/output')))\n\
             try:\n    \
                 open('/input/new', 'w')\n\
             except OSError as e:\n    \
                 print(type(e).__name__)",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "in ['data']\n\
         [('/output', ['log']), ('/output/copy', []), ('/output/copy/b', ['result.txt'])]\n\
         OSError\n",
        "{}",
        r.stderr
    );
    let output = sandbox.output_path().unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(output.join("log")).unwrap(),
        "one TWO"
    );
    assert_eq!(
        std::fs::read_to_string(output.join("copy/b/result.txt")).unwrap(),
        "atomic"
    );
}

/// A directory cannot be moved where what it holds would be deeper than
/// the 64 levels `/output` may have.
#[test]
fn directories_cannot_be_moved_too_deep() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let r = sandbox
        .run(
            "import errno, os\n\
             deep = '/output/' + '/'.join(['z'] * 63)\n\
             os.makedirs(deep)\n\
             os.makedirs('/output/t/u')\n\
             try:\n    \
                 os.rename('/output/t', deep + '/t')\n\
             except OSError as e:\n    \
                 print(errno.errorcode[e.errno])\n\
             os.rename('/output/t', os.path.dirname(deep) + '/t')\n\
             print('moved')",
        )
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_str()),
        (0, "EINVAL\nmoved\n"),
        "{}",
        r.stderr
    );
}

/// Without read permission `/output` is written, not read.
#[test]
fn write_only_output_is_written_not_read() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .output_dir(
            output.path(),
            hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run(
            "import os\n\
             open('/output/f', 'w').write('x')\n\
             print(os.access('/output/f', os.R_OK), os.access('/output/f', os.W_OK))\n\
             for op in (lambda: open('/output/f').read(), lambda: os.listdir('/output')):\n    \
                 try:\n        \
                     op()\n        \
                     print('read')\n    \
                 except PermissionError:\n        \
                     print('refused')",
        )
        .unwrap();
    assert_eq!(r.stdout, "False True\nrefused\nrefused\n", "{}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(output.path().join("f")).unwrap(),
        "x"
    );
}

#[test]
fn links_cannot_be_made_in_output() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let r = sandbox
        .run(
            "import os\n\
             open('/output/a', 'w').write('a')\n\
             for make in (lambda: os.symlink('../../../../etc/passwd', '/output/escape'),\n             \
                          lambda: os.link('/output/a', '/output/b')):\n    \
                 try:\n        \
                     make()\n        \
                     print('made')\n    \
                 except PermissionError:\n        \
                     print('refused')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "refused\nrefused\n", "{}", r.stderr);
    assert_eq!(sandbox.get_output_files().unwrap(), ["a"]);
}

#[test]
fn file_times_reach_the_host() {
    let input = tempfile::tempdir().unwrap();
    std::fs::write(input.path().join("in"), "in").unwrap();
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), Some(input.path())) else {
        return;
    };
    let r = sandbox
        .run(
            "import os\n\
             open('/output/a', 'w').write('a')\n\
             os.utime('/output/a', (1000000000, 1234567890))\n\
             s = os.stat('/output/a')\n\
             print(int(s.st_atime), int(s.st_mtime))\n\
             os.mkdir('/output/d')\n\
             os.utime('/output/d', (0, 1500000000))\n\
             print(int(os.stat('/output/d').st_mtime))\n\
             print(os.stat('/input/in').st_mtime > 0)\n\
             try:\n    os.utime('/input/in', (0, 0))\nexcept OSError:\n    print('refused')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout, "1000000000 1234567890\n1500000000\nTrue\nrefused\n",
        "{}",
        r.stderr
    );
    let output = sandbox.output_path().unwrap().unwrap();
    let modified = std::fs::metadata(output.join("a"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        modified,
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1234567890)
    );
}

/// This host's address on the interface it reaches the internet by: not
/// loopback, which guest sockets may never reach.
fn host_ip() -> Option<std::net::IpAddr> {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect("8.8.8.8:53").ok()?;
    Some(probe.local_addr().ok()?.ip())
}

/// A connection the guest opened stays open when the allowlist changes:
/// the change is checked at the next connect, without booting the guest
/// again (which would drop its sockets). A name that does not resolve is
/// no obstacle either, as nothing is resolved until the guest asks.
#[test]
fn native_network_follows_the_allowlist_without_a_reboot() {
    use std::io::Read as _;
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let Some(ip) = host_ip() else {
        eprintln!("skipping: no outbound interface");
        return;
    };
    let first = std::net::TcpListener::bind((ip, 0)).unwrap();
    let second = std::net::TcpListener::bind((ip, 0)).unwrap();
    let (p1, p2) = (
        first.local_addr().unwrap().port(),
        second.local_addr().unwrap().port(),
    );
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .guest(Unikraft::new().native_network(true))
        .build()
        .unwrap();
    let connect = |port: u16| {
        format!(
            "import errno, socket\n\
             def connect(port):\n    \
                 s = socket.socket()\n    \
                 s.settimeout(5)\n    \
                 try:\n        \
                     s.connect(('{ip}', port))\n        \
                     return s\n    \
                 except OSError as e:\n        \
                     print('refused' if e.errno == errno.EACCES else repr(e))\n\
             kept = connect({port})\n\
             print('connected' if kept else 'not connected')"
        )
    };
    let r = sandbox.run(&connect(p1)).unwrap();
    assert_eq!(r.stdout, "refused\nnot connected\n", "{}", r.stderr);

    sandbox
        .allow_domain(
            &format!("http://{ip}:{p1}"),
            hyperlight_sandbox::MethodFilter::Any,
        )
        .unwrap();
    let r = sandbox.run(&connect(p1)).unwrap();
    assert_eq!(r.stdout, "connected\n", "{}", r.stderr);
    let (mut held, _) = first.accept().unwrap();

    sandbox
        .allow_domain(
            "https://does-not-resolve.invalid",
            hyperlight_sandbox::MethodFilter::Any,
        )
        .unwrap();
    sandbox
        .allow_domain(
            &format!("http://{ip}:{p2}"),
            hyperlight_sandbox::MethodFilter::Any,
        )
        .unwrap();
    // The connection from before the change still carries data, and the
    // newly allowed port is reachable.
    let r = sandbox
        .run(&format!(
            "kept.sendall(b'still open')\n\
             other = connect({p2})\n\
             print('second connected' if other else 'second not connected')"
        ))
        .unwrap();
    assert_eq!(r.stdout, "second connected\n", "{}", r.stderr);
    assert!(r.stderr.is_empty(), "{}", r.stderr);
    let mut got = [0u8; 10];
    held.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    held.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"still open");
    // A port on the same host that no entry names is still refused.
    let r = sandbox.run(&format!("connect({})", p2 ^ 1)).unwrap();
    assert_eq!(r.stdout, "refused\n", "{}", r.stderr);
}

#[test]
fn output_trees_stop_at_the_walk_limits() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let output = sandbox.output_path().unwrap().unwrap();
    // Deeper than the 64 levels CapFs walks: refused as it is made.
    let r = sandbox
        .run(
            "import errno, os\n\
             try:\n    \
                 os.makedirs('/output/' + '/'.join(['d'] * 70))\n\
             except OSError as e:\n    \
                 print(errno.errorcode[e.errno])",
        )
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_str()),
        (0, "EINVAL\n"),
        "{}",
        r.stderr
    );
    std::fs::remove_dir_all(output.join("d")).unwrap();

    // More entries than the 100,000 it walks, directories counting:
    // refused from the one past them.
    let r = sandbox
        .run(
            "import errno, os\n\
             try:\n    \
                 for i in range(100100):\n        \
                     os.mkdir(f'/output/{i}')\n\
             except OSError as e:\n    \
                 print(i, errno.errorcode[e.errno])",
        )
        .unwrap();
    assert_eq!(
        (r.exit_code, r.stdout.as_str()),
        (0, "100000 EDQUOT\n"),
        "{}",
        r.stderr
    );
    for entry in std::fs::read_dir(&output).unwrap() {
        std::fs::remove_dir(entry.unwrap().path()).unwrap();
    }
    // The next run is unaffected.
    let r = sandbox.run("open('/output/ok', 'w').write('ok')").unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(sandbox.get_output_files().unwrap(), vec!["ok".to_string()]);
}

#[test]
fn exit_statuses_follow_cpython() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    for (code, status, stderr) in [
        ("None", 0, ""),
        ("True", 1, ""),
        ("False", 0, ""),
        ("-1", 255, ""),
        ("256", 0, ""),
        ("2 ** 70", 255, ""),
        ("'bye'", 1, "bye\n"),
    ] {
        let r = sandbox
            .run(&format!("import sys\nsys.exit({code})"))
            .unwrap();
        assert_eq!((r.exit_code, r.stderr.as_str()), (status, stderr), "{code}");
    }
}

#[test]
fn host_directories_in_output_are_kept() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(output.path().join("plots/png")).unwrap();
    let host_side = output.path().to_owned();
    let mut tools = ToolRegistry::new();
    tools.register_unvalidated("spoil", move |_| {
        std::fs::write(host_side.join("big.bin"), vec![0u8; 2048]).unwrap();
        Ok(serde_json::Value::Null)
    });
    let limits = hyperlight_sandbox::FilesystemLimits::new(1024, 8192, 8).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .with_tools(tools)
        .filesystem_limits(limits)
        .output_dir(
            output.path(),
            hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::READ | hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let r = sandbox
        .run(
            "import os\n\
             open('/output/plots/png/p.png', 'w').write('p')\n\
             os.makedirs('/output/guest/deep')\n\
             open('/output/guest/deep/f', 'w').write('f')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    // Kept from run to run.
    sandbox.run("pass").unwrap();
    assert!(output.path().join("plots/png/p.png").is_file());
    assert!(output.path().join("guest/deep/f").is_file());
    // A directory the host stages after the build.
    std::fs::create_dir(output.path().join("staged")).unwrap();
    // A run that fails the check clears it: the directories it started
    // with stay, emptied; the ones it made go.
    let r = sandbox
        .run("import os\nos.makedirs('/output/during/run')\ncall_tool('spoil')")
        .unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(output.path().join("plots/png").is_dir());
    assert!(!output.path().join("plots/png/p.png").exists());
    assert!(output.path().join("guest/deep").is_dir());
    assert!(!output.path().join("guest/deep/f").exists());
    assert!(output.path().join("staged").is_dir());
    assert!(!output.path().join("during").exists());
    assert!(!output.path().join("big.bin").exists());
}

#[test]
fn output_the_host_stages_reaches_the_guest() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .output_dir(
            output.path(),
            hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::READ | hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    std::fs::write(output.path().join("staged.txt"), "from the host").unwrap();
    let r = sandbox
        .run("print(open('/output/staged.txt').read())")
        .unwrap();
    assert_eq!(r.stdout, "from the host\n", "{}", r.stderr);
    // Restoring a snapshot clears what the host staged too.
    let snapshot = sandbox.snapshot().unwrap();
    sandbox.restore(&snapshot).unwrap();
    assert!(!output.path().join("staged.txt").exists());
}

#[test]
fn a_run_is_refused_on_what_the_host_put_in_output_past_the_rules() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let output = tempfile::tempdir().unwrap();
    let limits = hyperlight_sandbox::FilesystemLimits::new(1024, 4096, 4).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .filesystem_limits(limits)
        .output_dir(
            output.path(),
            hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::READ | hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    let code = "open('/output/ran', 'w').write('ran')";

    // Past the limits, as the other backends refuse it.
    std::fs::write(output.path().join("big.bin"), vec![0u8; 2048]).unwrap();
    let error = sandbox.run(code).unwrap_err();
    assert!(
        format!("{error:#}").contains("refusing to run"),
        "{error:#}"
    );
    assert!(!output.path().join("ran").exists());
    // Left as the host put it.
    assert_eq!(
        std::fs::read(output.path().join("big.bin")).unwrap().len(),
        2048
    );
    std::fs::remove_file(output.path().join("big.bin")).unwrap();

    // A link, which the guest would follow through its mount.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", output.path().join("link")).unwrap();
        let error = sandbox.run(code).unwrap_err();
        assert!(format!("{error:#}").contains("symbolic links"), "{error:#}");
        assert!(!output.path().join("ran").exists());
        std::fs::remove_file(output.path().join("link")).unwrap();
    }

    // Put right, the next run goes ahead.
    assert_eq!(sandbox.run(code).unwrap().exit_code, 0);
    assert!(output.path().join("ran").is_file());
}

/// Guest sockets reach an allowed name on its scheme's port, by the names
/// DNS gives the guest, and nothing else.  Needs the internet; skips
/// without it.
#[test]
fn native_network_reaches_allowed_names_by_name() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    if std::net::ToSocketAddrs::to_socket_addrs(&("example.com", 80)).is_err() {
        eprintln!("skipping: example.com does not resolve (no internet?)");
        return;
    }
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .guest(Unikraft::new().native_network(true))
        .build()
        .unwrap();
    sandbox
        .allow_domain("http://example.com", hyperlight_sandbox::MethodFilter::Any)
        .unwrap();
    let r = sandbox
        .run(
            "import errno, socket, urllib.request\n\
             print('allowed', urllib.request.urlopen('http://example.com', timeout=15).status)\n\
             s = socket.socket()\n\
             s.settimeout(5)\n\
             try:\n    \
                 s.connect(('example.com', 443))\n    \
                 print('443 connected')\n\
             except OSError as e:\n    \
                 print('443', 'refused' if e.errno == errno.EACCES else repr(e))\n\
             try:\n    \
                 socket.getaddrinfo('github.com', 80)\n    \
                 print('other name resolved')\n\
             except OSError:\n    \
                 print('other name refused')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout, "allowed 200\n443 refused\nother name refused\n",
        "{}",
        r.stderr
    );
}

/// The guest's `/output` is the directory `CapFs` opened, which serves it:
/// replacing the path on the host afterwards sends neither to the new one.
#[test]
fn a_replaced_output_directory_changes_nothing() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let base = tempfile::tempdir().unwrap();
    let output = base.path().join("out");
    std::fs::create_dir(&output).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .output_dir(
            &output,
            hyperlight_sandbox::DirPerms::READ | hyperlight_sandbox::DirPerms::MUTATE,
            hyperlight_sandbox::FilePerms::READ | hyperlight_sandbox::FilePerms::WRITE,
        )
        .guest(Unikraft::new())
        .build()
        .unwrap();
    assert_eq!(sandbox.run("pass").unwrap().exit_code, 0);

    // The host swaps the directory for another of the same name.
    let kept = base.path().join("kept");
    std::fs::rename(&output, &kept).unwrap();
    std::fs::create_dir(&output).unwrap();
    let r = sandbox
        .run("open('/output/a.txt', 'w').write('a')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert!(kept.join("a.txt").is_file());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    // And it is the one cleared.
    let snapshot = sandbox.snapshot().unwrap();
    sandbox.restore(&snapshot).unwrap();
    assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 0);
}

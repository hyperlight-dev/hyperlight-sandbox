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

#[test]
fn output_limits_are_enforced_after_the_run() {
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
        .run("open('/output/big.bin', 'wb').write(b'0' * 2048)")
        .unwrap();
    assert_eq!(r.exit_code, 1);
    assert!(
        r.stderr.contains("exceeded the filesystem limits"),
        "{}",
        r.stderr
    );
    assert!(sandbox.get_output_files().unwrap().is_empty());
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

#[test]
fn out_of_range_exit_status_keeps_output_and_limits() {
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
            "open('/output/big.bin', 'wb').write(b'0' * 2048)\n\
             print('before')\nimport sys\nsys.exit(2 ** 40)",
        )
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

#[test]
fn symlinks_in_output_are_removed() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let r = sandbox
        .run(
            "import os\n\
             os.symlink('../../../../etc/passwd', '/output/escape')\n\
             open('/output/ok.txt', 'w').write('ok')",
        )
        .unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("symbolic links"), "{}", r.stderr);
    let output = sandbox.output_path().unwrap().unwrap();
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn unresolvable_hosts_do_not_break_native_network() {
    let Some(rootfs) = rootfs("python") else {
        return;
    };
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs)
        .guest(Unikraft::new().native_network(true))
        .build()
        .unwrap();
    sandbox
        .allow_domain(
            "https://does-not-resolve.invalid",
            hyperlight_sandbox::MethodFilter::Any,
        )
        .unwrap();
    let r = sandbox.run("print('first')").unwrap();
    assert_eq!((r.exit_code, r.stdout.as_str()), (0, "first\n"));
    assert!(
        r.stderr.contains("does-not-resolve.invalid"),
        "{}",
        r.stderr
    );
    // Said once, and later runs are unaffected.
    let r = sandbox.run("print('second')").unwrap();
    assert_eq!((r.exit_code, r.stdout.as_str()), (0, "second\n"));
    assert!(r.stderr.is_empty(), "{}", r.stderr);
}

#[test]
fn output_trees_past_the_walk_limits_fail_and_are_removed() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let output = sandbox.output_path().unwrap().unwrap();
    for code in [
        // Deeper than the 64 levels CapFs walks.
        "import os\nos.makedirs('/output/' + '/'.join(['d'] * 70))",
        // More entries than the 100,000 it walks, directories counting.
        "import os\nfor i in range(100100):\n    os.mkdir(f'/output/{i}')",
    ] {
        let r = sandbox.run(code).unwrap();
        assert_eq!(r.exit_code, 1, "{}", r.stderr);
        assert!(r.stderr.contains("its files were removed"), "{}", r.stderr);
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
        // The next run is unaffected.
        let r = sandbox.run("open('/output/ok', 'w').write('ok')").unwrap();
        assert_eq!(r.exit_code, 0, "{}", r.stderr);
        assert_eq!(sandbox.get_output_files().unwrap(), vec!["ok".to_string()]);
    }
}

#[test]
fn hard_links_in_output_are_removed() {
    let Some(mut sandbox) = python_sandbox(ToolRegistry::new(), None) else {
        return;
    };
    let r = sandbox
        .run("import os\nopen('/output/a', 'w').write('a')\nos.link('/output/a', '/output/b')")
        .unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("hard-linked"), "{}", r.stderr);
    assert!(sandbox.get_output_files().unwrap().is_empty());
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
        .run("import os\nos.makedirs('/output/during/run')\nos.symlink('x', '/output/link')")
        .unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(output.path().join("plots/png").is_dir());
    assert!(!output.path().join("plots/png/p.png").exists());
    assert!(output.path().join("guest/deep").is_dir());
    assert!(!output.path().join("guest/deep/f").exists());
    assert!(output.path().join("staged").is_dir());
    assert!(!output.path().join("during").exists());
    assert!(!output.path().join("link").exists());
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

#[test]
fn a_replaced_output_directory_is_refused() {
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
        .guest(Unikraft::new().native_network(true))
        .build()
        .unwrap();
    assert_eq!(sandbox.run("pass").unwrap().exit_code, 0);

    // The host swaps the directory for another of the same name.
    let kept = base.path().join("kept");
    std::fs::rename(&output, &kept).unwrap();
    std::fs::create_dir(&output).unwrap();
    sandbox
        .allow_domain("http://127.0.0.1", hyperlight_sandbox::MethodFilter::Any)
        .unwrap();
    let code = "import os\nos.symlink('x', '/output/link')";
    let error = sandbox.run(code).unwrap_err();
    assert!(format!("{error:#}").contains("was replaced"), "{error:#}");
    // Nothing reached the new directory unchecked.
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);

    // Putting the original back lets the re-boot, and the run, go ahead.
    std::fs::remove_dir(&output).unwrap();
    std::fs::rename(&kept, &output).unwrap();
    let r = sandbox.run(code).unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("symbolic links"), "{}", r.stderr);
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

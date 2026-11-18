use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;

const BOT: &str = env!("CARGO_BIN_EXE_ai-bot");

#[test]
fn help_shows_all_three_service_flags() {
    let output = Command::new(BOT).arg("--help").output().expect("读取帮助");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8");
    for flag in ["--start", "--stop", "--restart"] {
        assert!(help.contains(flag), "{help}");
    }
}

#[test]
fn service_flags_control_the_installed_unit_and_propagate_errors() {
    let dir = tempfile::tempdir().expect("临时目录");
    let systemctl = dir.path().join("systemctl");
    fs::write(
        &systemctl,
        r#"#!/bin/sh
case "$*" in
    '--user show --property=LoadState --value ai-bot.service') echo "$USER_UNIT" ;;
    '--system show --property=LoadState --value ai-bot.service') echo "$SYSTEM_UNIT" ;;
    *) echo "${0##*/} $*"; exit "$ACTION_STATUS" ;;
esac
"#,
    )
    .expect("写入 systemctl");
    fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o755)).expect("执行权限");
    fs::hard_link(&systemctl, dir.path().join("sudo")).expect("模拟 sudo");
    let run = |flags: &[&str], user: &str, system: &str, code: &str| {
        Command::new(BOT)
            .args(flags)
            .env("PATH", dir.path())
            .env("USER_UNIT", user)
            .env("SYSTEM_UNIT", system)
            .env("ACTION_STATUS", code)
            .output()
            .expect("执行 ai-bot")
    };
    for (flag, action) in [
        ("--start", "start"),
        ("--stop", "stop"),
        ("--restart", "restart"),
    ] {
        for (user, system, prefix) in [
            ("loaded", "loaded", "systemctl --user"),
            ("not-found", "loaded", "sudo systemctl"),
        ] {
            let output = run(&[flag], user, system, "0");
            assert!(output.status.success());
            assert_eq!(
                output.stdout,
                format!("{prefix} {action} ai-bot.service\n").as_bytes()
            );
        }
    }
    let failed = run(&["--restart"], "loaded", "loaded", "42");
    assert_eq!(failed.status.code(), Some(42));
    assert_eq!(failed.stdout, b"systemctl --user restart ai-bot.service\n");
    for output in [
        run(&["--stop"], "not-found", "not-found", "0"),
        run(&["--start", "--stop"], "loaded", "loaded", "0"),
    ] {
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

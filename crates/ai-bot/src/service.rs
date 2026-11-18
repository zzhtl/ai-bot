use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitCode};

const UNIT: &str = "ai-bot.service";

pub fn control(action: &str) -> ExitCode {
    // 按 LoadState 查找，已停止的服务也能被 --start 找到。
    for scope in ["--user", "--system"] {
        let probe = Command::new("systemctl")
            .args([scope, "show", "--property=LoadState", "--value", UNIT])
            .output();
        let state = match probe {
            Ok(output) => output,
            Err(err) => {
                eprintln!("无法执行 systemctl：{err}");
                return ExitCode::FAILURE;
            }
        };
        if !state.status.success() {
            if scope == "--user" {
                continue; // 系统级部署可以没有用户 systemd 会话
            }
            eprint!("{}", String::from_utf8_lossy(&state.stderr));
            return ExitCode::FAILURE;
        }
        if matches!(state.stdout.trim_ascii(), b"not-found" | b"") {
            continue;
        }
        let (program, first_arg) = if scope == "--user" {
            ("systemctl", "--user")
        } else {
            ("sudo", "systemctl")
        };
        let err = Command::new(program).args([first_arg, action, UNIT]).exec();
        eprintln!("无法执行 {program} {first_arg} {action} {UNIT}：{err}");
        return ExitCode::FAILURE;
    }
    eprintln!("未找到 {UNIT}，请在部署机上用部署账号执行");
    ExitCode::FAILURE
}

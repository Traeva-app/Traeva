//! Developer driver for the unregistered module; not a product supervisor.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use traeva_pdf_ocr_core_check::pdf_ocr;

#[global_allocator]
static ALLOCATOR: traeva_pdf_ocr_core_check::pdf_child::CappedAlloc =
    traeva_pdf_ocr_core_check::pdf_child::CappedAlloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--inspect")
        && let Some(position) = args.iter().position(|s| s == "--parent-pid")
    {
        let parent = args
            .get(position + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if pdf_ocr::child_parent_watchdog(parent).is_err() {
            std::process::exit(4);
        }
    }
    let code = match args.first().map(String::as_str) {
        Some("--prepare") => pdf_ocr::ocr_prepare_main(&args[1..]),
        Some("--recognize") => pdf_ocr::ocr_child_main(&args[1..]),
        Some("--inspect") => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take((20 << 20) + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            if bytes.len() > 20 << 20 {
                2
            } else {
                let result = pdf_ocr::read_with_ocr(&bytes, &pdf_ocr::Settings::default());
                serde_json::to_writer(std::io::stdout(), &result).unwrap();
                0
            }
        }
        Some("--monitor") => {
            monitor(&args[1..]);
            0
        }
        _ => 2,
    };
    std::process::exit(code);
}

struct Capture {
    bytes: Vec<u8>,
    overflow: bool,
    io_error: bool,
}

fn capture(mut pipe: impl Read) -> Capture {
    let mut capture = Capture {
        bytes: Vec::new(),
        overflow: false,
        io_error: false,
    };
    let mut chunk = [0; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let keep = n.min((8_000_000usize).saturating_sub(capture.bytes.len()));
                capture.bytes.extend_from_slice(&chunk[..keep]);
                capture.overflow |= keep < n;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                capture.io_error = true;
                break;
            }
        }
    }
    capture
}

fn captured_answer(
    outcome: &mut &'static str,
    stdout: &Capture,
    stderr: &Capture,
) -> Option<serde_json::Value> {
    if *outcome != "finished" {
        return None;
    }
    if stdout.io_error || stderr.io_error {
        *outcome = "io_error";
        return None;
    }
    if stdout.overflow || stderr.overflow {
        *outcome = "output_limit";
        return None;
    }
    match serde_json::from_slice::<serde_json::Value>(&stdout.bytes) {
        Ok(answer) if answer.is_object() => Some(answer),
        _ => {
            *outcome = "protocol_error";
            None
        }
    }
}

fn exited(pid: u32) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    assert_eq!(result, 0, "waitid failed");
    info.si_pid != 0
        && matches!(
            info.si_code,
            libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
        )
}

fn monitor(args: &[String]) {
    assert!(args.len() >= 4);
    let started = Instant::now();
    let timeout = Duration::from_millis(args[2].parse().unwrap());
    let cap: u64 = args[3].parse::<u64>().unwrap() << 20;
    let cancel_after = args
        .get(4)
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis);
    let mut bytes = Vec::new();
    if args[1] != "--prepare" {
        std::io::stdin()
            .take((20 << 20) + 1)
            .read_to_end(&mut bytes)
            .unwrap();
    }
    let mut child = Command::new(&args[0])
        .arg(&args[1])
        .args(["--parent-pid", &std::process::id().to_string()])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut input = child.stdin.take().unwrap();
    let writer = thread::spawn(move || input.write_all(&bytes).is_ok());
    let output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let reader = thread::spawn(move || capture(output));
    let error_reader = thread::spawn(move || capture(errors));
    let mut peak = 0;
    let mut sampled_peak = 0;
    let mut max_gap = Duration::ZERO;
    let mut last = started;
    let mut samples = 0;
    let mut outcome = loop {
        if exited(pid) {
            break "finished";
        }
        let now = Instant::now();
        max_gap = max_gap.max(now.duration_since(last));
        last = now;
        if cancel_after.is_some_and(|limit| started.elapsed() >= limit) {
            break "cancelled";
        }
        if started.elapsed() >= timeout {
            break "time_limit";
        }
        let Some(memory) = pdf_ocr::process_memory(pid) else {
            break "monitor_error";
        };
        samples += 1;
        sampled_peak = sampled_peak.max(memory.footprint_bytes);
        peak = peak.max(memory.lifetime_peak_bytes);
        if memory.footprint_bytes > cap || memory.lifetime_peak_bytes > cap {
            break "memory_limit";
        }
        thread::sleep(Duration::from_millis(5));
    };
    let stop = (outcome != "finished").then(Instant::now);
    if stop.is_some() {
        child.kill().unwrap();
    }
    let cleanup = Instant::now();
    while !exited(pid) && cleanup.elapsed() < Duration::from_secs(2) {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(exited(pid), "child failed to exit within bounded cleanup");
    max_gap = max_gap.max(last.elapsed());
    let kill_to_exit = stop.map(|t| t.elapsed().as_secs_f64() * 1000.0);
    let final_memory = pdf_ocr::process_memory(pid);
    let final_kernel_peak = final_memory.map(|memory| memory.kernel_lifetime_peak_bytes);
    if let Some(memory) = final_memory {
        peak = peak.max(memory.lifetime_peak_bytes);
        if outcome == "finished" && peak > cap {
            outcome = "memory_limit";
        }
    } else if outcome == "finished" {
        outcome = "monitor_error";
    }
    let status = child.wait().unwrap();
    let input_written = writer.join().unwrap();
    let stdout = reader.join().unwrap();
    let stderr = error_reader.join().unwrap();
    if outcome == "finished" && (!status.success() || !input_written) {
        outcome = "child_failed";
    }
    if outcome == "finished" && started.elapsed() >= timeout {
        outcome = "time_limit";
    }
    if outcome == "finished" && cancel_after.is_some_and(|limit| started.elapsed() >= limit) {
        outcome = "cancelled";
    }
    let mut answer = captured_answer(&mut outcome, &stdout, &stderr);
    // Parsing is part of the budget. Completion after this final check
    // wins over later cancellation, matching the experiment supervisor.
    if outcome == "finished" && started.elapsed() >= timeout {
        outcome = "time_limit";
        answer = None;
    }
    if outcome == "finished" && cancel_after.is_some_and(|limit| started.elapsed() >= limit) {
        outcome = "cancelled";
        answer = None;
    }
    let record = serde_json::json!({"outcome":outcome,"answer":answer,
        "exit_code":status.code(),"child_pid":pid,"child_reaped":true,
        "wall_ms":started.elapsed().as_secs_f64()*1000.0,"peak_footprint_bytes":peak,
        "sampled_peak_bytes":sampled_peak,"final_kernel_lifetime_peak_bytes":final_kernel_peak,
        "over_budget_bytes":peak.saturating_sub(cap),
        "kill_to_exit_ms":kill_to_exit,"samples":samples,"max_observation_gap_ms":max_gap.as_secs_f64()*1000.0,
        "stdout_io_error":stdout.io_error,"stderr_io_error":stderr.io_error,
        "raw_stdout":String::from_utf8_lossy(&stdout.bytes),"stderr":String::from_utf8_lossy(&stderr.bytes)});
    serde_json::to_writer(std::io::stdout(), &record).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InterruptedThenData {
        interrupted: bool,
        bytes: std::io::Cursor<Vec<u8>>,
    }

    impl Read for InterruptedThenData {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.bytes.read(buffer)
        }
    }

    struct DataThenError {
        bytes: std::io::Cursor<Vec<u8>>,
    }

    impl Read for DataThenError {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let size = self.bytes.read(buffer)?;
            if size == 0 {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(size)
            }
        }
    }

    #[test]
    fn interrupted_capture_retries_and_retains_the_complete_answer() {
        let stdout = capture(InterruptedThenData {
            interrupted: false,
            bytes: std::io::Cursor::new(br#"{"status":"ready"}"#.to_vec()),
        });
        let mut outcome = "finished";
        let answer = captured_answer(&mut outcome, &stdout, &capture(std::io::empty()));
        assert_eq!(outcome, "finished");
        assert_eq!(answer.unwrap()["status"], "ready");
        assert!(!stdout.io_error);
    }

    #[test]
    fn a_read_error_rejects_even_an_apparently_complete_json_answer() {
        let errored = capture(DataThenError {
            bytes: std::io::Cursor::new(br#"{"status":"ready"}"#.to_vec()),
        });
        let valid = capture(br#"{"status":"ready"}"#.as_slice());
        let empty = capture(std::io::empty());
        for (stdout, stderr) in [(&errored, &empty), (&valid, &errored)] {
            let mut outcome = "finished";
            assert!(captured_answer(&mut outcome, stdout, stderr).is_none());
            assert_eq!(outcome, "io_error");
        }
    }

    #[test]
    fn malformed_or_non_object_output_is_a_protocol_failure() {
        for bytes in [b"{".as_slice(), b"null", b"[]", b"{} trailing bytes", b""] {
            let mut outcome = "finished";
            assert!(
                captured_answer(&mut outcome, &capture(bytes), &capture(std::io::empty()))
                    .is_none()
            );
            assert_eq!(outcome, "protocol_error");
        }
    }
}

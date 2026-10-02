//! Strict validation for headless Godot tasks.

use anyhow::{Result, ensure};
use std::process::Output;

/// Checks applied to captured Godot output. Project-specific prefixes and
/// success markers are supplied by the caller. Shutdown leaks are rejected by
/// default; opting out only permits the two known shutdown diagnostics.
#[derive(Clone, Debug)]
pub struct OutputPolicy<'a> {
    pub success_marker: Option<&'a str>,
    pub error_prefixes: &'a [&'a str],
    pub allow_shutdown_leaks: bool,
}

impl Default for OutputPolicy<'_> {
    fn default() -> Self {
        Self {
            success_marker: None,
            error_prefixes: &["SCRIPT ERROR", "ERROR:"],
            allow_shutdown_leaks: false,
        }
    }
}

impl OutputPolicy<'_> {
    pub fn contains_error(&self, output: &str) -> bool {
        output.lines().any(|line| {
            let line = line.trim_start();
            self.error_prefixes
                .iter()
                .any(|prefix| line.starts_with(prefix))
                && !(self.allow_shutdown_leaks && is_known_shutdown_leak(line))
        })
    }

    pub fn ensure_success(&self, output: &Output, label: &str) -> Result<()> {
        crate::process::ensure_success(output, label)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        ensure!(
            !self.contains_error(&stdout) && !self.contains_error(&stderr),
            "{label} reported an error despite a zero exit status\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        if let Some(expected) = self.success_marker {
            ensure!(
                stdout.contains(expected),
                "{label} did not emit success marker {expected:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
        }
        Ok(())
    }
}

fn is_known_shutdown_leak(line: &str) -> bool {
    (line.starts_with("ERROR: ")
        && line.contains(" RID allocations of type '")
        && line.ends_with(" were leaked at exit."))
        || (line.starts_with("ERROR: ")
            && line.ends_with(" resources still in use at exit (run with --verbose for details)."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(code << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code as u32)
        };
        Output {
            status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn zero_exit_and_marker_do_not_hide_errors_on_either_stream() {
        let policy = OutputPolicy {
            success_marker: Some("VALID"),
            ..Default::default()
        };
        for (stdout, stderr) in [
            ("VALID\n  SCRIPT ERROR: failure", ""),
            ("VALID", "ERROR: failure"),
        ] {
            assert!(
                policy
                    .ensure_success(&output(0, stdout, stderr), "fixture")
                    .is_err()
            );
        }
        assert!(
            policy
                .ensure_success(&output(0, "VALID", ""), "fixture")
                .is_ok()
        );
        assert!(
            policy
                .ensure_success(&output(0, "", "VALID"), "fixture")
                .is_err()
        );
        let error = policy
            .ensure_success(&output(1, "VALID", "details"), "fixture")
            .unwrap_err()
            .to_string();
        assert!(error.contains("VALID") && error.contains("details"));
    }

    #[test]
    fn project_prefixes_are_configurable_and_leak_allowance_is_narrow() {
        let policy = OutputPolicy {
            error_prefixes: &["SCRIPT ERROR", "ERROR:", "FIXTURE_ERROR:"],
            allow_shutdown_leaks: true,
            ..Default::default()
        };
        let leaks = "ERROR: 9 RID allocations of type 'DummyTexture' were leaked at exit.\nERROR: 31 resources still in use at exit (run with --verbose for details).";
        assert!(OutputPolicy::default().contains_error(leaks));
        assert!(!policy.contains_error(leaks));
        for error in [
            "SCRIPT ERROR: failure",
            "FIXTURE_ERROR: failure",
            "ERROR: Failed loading resource",
        ] {
            assert!(policy.contains_error(error));
        }
        assert!(!OutputPolicy::default().contains_error("FIXTURE_ERROR: failure"));
    }
}

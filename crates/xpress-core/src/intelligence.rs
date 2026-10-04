//! Apple Intelligence: rewrite, proofread, summarise and translate text with
//! the on-device model (macOS 26+, Apple silicon, Apple Intelligence on).
//!
//! The model is only reachable from Swift, so the app bundles a small helper,
//! `xpress-ai` (tools/xpress-ai), and runs it per request: `status` reports
//! whether the model can be used, `respond --instructions …` reads the text on
//! stdin and prints the answer. Nothing leaves the Mac.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::tools::{self, Tool};

/// Text longer than this is cut (the model's context is about 4,000 tokens).
pub const MAX_INPUT_CHARS: usize = 8_000;

/// What to do with the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Task {
    Summarize,
    Shorter,
    Professional,
    Friendly,
    Proofread,
    TranslateToEnglish,
}

impl Task {
    pub const ALL: [Task; 6] = [
        Task::Summarize,
        Task::Proofread,
        Task::Shorter,
        Task::Professional,
        Task::Friendly,
        Task::TranslateToEnglish,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Task::Summarize => "Summarise",
            Task::Shorter => "Make shorter",
            Task::Professional => "Make professional",
            Task::Friendly => "Make friendly",
            Task::Proofread => "Proofread",
            Task::TranslateToEnglish => "Translate to English",
        }
    }

    /// The title of the result.
    pub fn result_title(self) -> &'static str {
        match self {
            Task::Summarize => "Summary",
            Task::Proofread => "Proofread",
            Task::TranslateToEnglish => "Translation",
            _ => "Rewritten",
        }
    }

    pub fn instructions(self) -> String {
        let what = match self {
            Task::Summarize => {
                "Summarise the user's text in a few short sentences or bullet points, \
                 keeping the key facts, names, numbers and dates."
            }
            Task::Shorter => {
                "Rewrite the user's text to be clearly shorter and more concise, \
                 keeping its meaning and tone."
            }
            Task::Professional => {
                "Rewrite the user's text so it sounds professional, clear and polite, \
                 keeping its meaning."
            }
            Task::Friendly => {
                "Rewrite the user's text so it sounds warm, friendly and relaxed, \
                 keeping its meaning."
            }
            Task::Proofread => {
                "Correct the spelling, grammar and punctuation of the user's text. Do not \
                 change its meaning, wording or style beyond what is needed."
            }
            Task::TranslateToEnglish => {
                "Translate the user's text into natural English, keeping its meaning, tone \
                 and formatting."
            }
        };
        format!(
            "{what} Keep the language of the text unless asked otherwise. Reply with the \
             result only — no introduction, no quotes, no notes."
        )
    }
}

/// Whether Apple Intelligence can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Available,
    /// Turned off in System Settings → Apple Intelligence & Siri.
    NotEnabled,
    /// This Mac can't run it (e.g. an Intel Mac).
    DeviceNotEligible,
    /// Still downloading or preparing the model.
    ModelNotReady,
    Unavailable,
    /// The helper isn't installed, or macOS is older than 26.
    Missing,
}

impl Status {
    pub fn parse(s: &str) -> Status {
        match s.trim() {
            "available" => Status::Available,
            "not-enabled" => Status::NotEnabled,
            "device-not-eligible" => Status::DeviceNotEligible,
            "model-not-ready" => Status::ModelNotReady,
            _ => Status::Unavailable,
        }
    }

    /// A short explanation for the user, when it isn't available.
    pub fn explain(self) -> Option<&'static str> {
        match self {
            Status::Available => None,
            Status::NotEnabled => {
                Some("Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri.")
            }
            Status::DeviceNotEligible => Some("This Mac doesn't support Apple Intelligence."),
            Status::ModelNotReady => {
                Some("Apple Intelligence is still getting ready — try again in a while.")
            }
            Status::Unavailable => Some("Apple Intelligence isn't available right now."),
            Status::Missing => Some("Apple Intelligence needs macOS 26 or later."),
        }
    }
}

/// Ask the helper whether the model is ready.
pub fn status() -> Status {
    let Ok(helper) = tools::resolve(Tool::XpressAi) else {
        return Status::Missing;
    };
    match run_helper(&helper, &["status"], None, Duration::from_secs(15)) {
        Ok(out) => Status::parse(&out),
        // An older macOS can't even start it (the framework is missing).
        Err(_) => Status::Missing,
    }
}

/// Run `task` on `text`. Long text is cut to [`MAX_INPUT_CHARS`].
pub fn run(task: Task, text: &str) -> Result<String, String> {
    let helper = tools::resolve(Tool::XpressAi).map_err(|e| e.to_string())?;
    let input = prepare_input(text);
    if input.trim().is_empty() {
        return Err("There's no text to work on.".into());
    }
    let out = run_helper(
        &helper,
        &["respond", "--instructions", &task.instructions()],
        Some(&input),
        Duration::from_secs(120),
    )?;
    let out = clean_output(&out);
    if out.is_empty() {
        Err("Apple Intelligence returned nothing.".into())
    } else {
        Ok(out)
    }
}

fn prepare_input(text: &str) -> String {
    text.trim().chars().take(MAX_INPUT_CHARS).collect()
}

/// Models sometimes wrap the answer in quotes or a code fence anyway.
fn clean_output(out: &str) -> String {
    let mut s = out.trim();
    if let Some(inner) = s.strip_prefix("```").and_then(|r| r.strip_suffix("```")) {
        s = inner
            .split_once('\n')
            .map_or(inner, |(_, body)| body)
            .trim();
    }
    for (open, close) in [('"', '"'), ('“', '”')] {
        if s.starts_with(open) && s.ends_with(close) && s.chars().count() > 1 {
            let inner = &s[open.len_utf8()..s.len() - close.len_utf8()];
            if !inner.contains(open) && !inner.contains(close) {
                s = inner.trim();
            }
        }
    }
    s.to_string()
}

fn run_helper(
    helper: &std::path::Path,
    args: &[&str],
    stdin: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new(helper)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start Apple Intelligence: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                return Err("Apple Intelligence took too long.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() {
            format!("Apple Intelligence failed ({})", out.status)
        } else {
            msg
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(Status::parse("available\n"), Status::Available);
        assert_eq!(Status::parse("not-enabled"), Status::NotEnabled);
        assert_eq!(
            Status::parse("device-not-eligible"),
            Status::DeviceNotEligible
        );
        assert_eq!(Status::parse("model-not-ready"), Status::ModelNotReady);
        assert_eq!(Status::parse("???"), Status::Unavailable);
        assert!(Status::Available.explain().is_none());
        assert!(Status::NotEnabled
            .explain()
            .unwrap()
            .contains("System Settings"));
    }

    #[test]
    fn every_task_asks_for_the_result_only() {
        for task in Task::ALL {
            let i = task.instructions();
            assert!(i.contains("Reply with the"), "{task:?}: {i}");
            assert!(!task.label().is_empty() && !task.result_title().is_empty());
        }
        assert!(Task::Proofread.instructions().contains("Keep the language"));
        assert!(Task::TranslateToEnglish.instructions().contains("English"));
    }

    #[test]
    fn input_is_trimmed_and_cut() {
        assert_eq!(prepare_input("  hi \n"), "hi");
        assert_eq!(
            prepare_input(&"é".repeat(MAX_INPUT_CHARS + 10))
                .chars()
                .count(),
            MAX_INPUT_CHARS
        );
    }

    #[test]
    fn output_is_cleaned() {
        assert_eq!(clean_output("  Hello there.\n"), "Hello there.");
        assert_eq!(clean_output("\"Hello there.\""), "Hello there.");
        assert_eq!(clean_output("“Hei.”"), "Hei.");
        assert_eq!(clean_output("```text\nfixed\n```"), "fixed");
        assert_eq!(
            clean_output("\"a\" and \"b\""),
            "\"a\" and \"b\"",
            "not one quote"
        );
    }
}

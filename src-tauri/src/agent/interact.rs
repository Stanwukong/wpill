//! Questions and plans (Phase 2.8c): `AskUserQuestion` and `ExitPlanMode`, held like any permission request but
//! answered with more than "allow". Shared by terminal sessions (`claude/ask.rs`, `claude/asks.rs`) and agent runs
//! (the prompt tool). Pure; unit-tested.
//!
//! An allow for these tools must carry the answer in `updatedInput` (hooks docs, "Tools that require user
//! interaction"): for a question, the original `questions` plus `answers` (question text → option label,
//! multi-select labels joined by ", "); for a plan, the input as Claude Code sent it. A bare allow is never built:
//! `answered_input` refuses until every question has an answer.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::ipc::{Interaction, QuestionOption, QuestionView};

pub const QUESTION_TOOL: &str = "AskUserQuestion";
pub const PLAN_TOOL: &str = "ExitPlanMode";
const MAX_QUESTIONS: usize = 4;
const MAX_OPTIONS: usize = 8;
const MAX_TEXT: usize = 2000;
/// A free-text answer (the terminal's "Other"), in characters.
pub const MAX_ANSWER: usize = 2000;
/// Feedback on a plan, in characters.
pub const MAX_FEEDBACK: usize = 8000;
const MAX_TITLE: usize = 80;

/// One of the two tools whose answer is more than allow / deny.
pub fn is_interactive(tool: &str) -> bool {
    tool == QUESTION_TOOL || tool == PLAN_TOOL
}

/// What the island shows for such a request, with the answers given so far; `None` for any other tool, or a
/// question whose input doesn't parse (it then goes to the terminal, or is denied as malformed in a run).
pub fn interaction(
    tool: &str,
    input: &Value,
    answers: &BTreeMap<String, String>,
) -> Option<Interaction> {
    match tool {
        QUESTION_TOOL => Some(Interaction::Question {
            questions: questions(input)?,
            answers: answers.clone(),
        }),
        PLAN_TOOL => {
            let plan = input.get("plan").and_then(Value::as_str).unwrap_or("");
            Some(Interaction::Plan {
                title: plan_title(plan),
                lines: plan.lines().count() as u32,
            })
        }
        _ => None,
    }
}

/// The questions of an `AskUserQuestion` input: 1–4, each with a question and 1–8 labelled options.
pub fn questions(input: &Value) -> Option<Vec<QuestionView>> {
    let list = input.get("questions")?.as_array()?;
    if list.is_empty() || list.len() > MAX_QUESTIONS {
        return None;
    }
    list.iter()
        .map(|q| {
            let text = |v: &Value, key: &str| {
                v.get(key)
                    .and_then(Value::as_str)
                    .filter(|s| s.chars().count() <= MAX_TEXT)
                    .map(str::to_string)
            };
            let question = text(q, "question").filter(|s| !s.trim().is_empty())?;
            let options = q.get("options")?.as_array()?;
            if options.is_empty() || options.len() > MAX_OPTIONS {
                return None;
            }
            let options = options
                .iter()
                .map(|o| {
                    Some(QuestionOption {
                        label: text(o, "label").filter(|s| !s.trim().is_empty())?,
                        description: text(o, "description").unwrap_or_default(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(QuestionView {
                header: text(q, "header").unwrap_or_default(),
                question,
                options,
                multi_select: q
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Your answer to one question, as Claude Code expects it: the chosen labels (joined by ", " for a multi-select),
/// or your own words (`other`). `None` if the question isn't asked, a label isn't one of its options, more than
/// one is chosen for a single-select, or the text is empty or too long.
pub fn answer(
    questions: &[QuestionView],
    question: &str,
    labels: &[String],
    other: Option<&str>,
) -> Option<(String, String)> {
    let q = questions.iter().find(|q| q.question == question)?;
    let other = other.map(str::trim).filter(|t| !t.is_empty());
    if let Some(text) = other {
        return (labels.is_empty() && text.chars().count() <= MAX_ANSWER)
            .then(|| (q.question.clone(), text.to_string()));
    }
    if labels.is_empty() || (!q.multi_select && labels.len() > 1) {
        return None;
    }
    // In the order the options are listed, once each.
    let chosen: Vec<&str> = q
        .options
        .iter()
        .filter(|o| labels.contains(&o.label))
        .map(|o| o.label.as_str())
        .collect();
    (chosen.len() == labels.len()).then(|| (q.question.clone(), chosen.join(", ")))
}

/// Whether every question has an answer.
pub fn complete(questions: &[QuestionView], answers: &BTreeMap<String, String>) -> bool {
    questions.iter().all(|q| answers.contains_key(&q.question))
}

/// The `updatedInput` for an answered question: `original` (as Claude Code sent it) plus `answers`. `None` unless
/// it's an `AskUserQuestion` input and every one of its questions is answered, with nothing else.
pub fn answered_input(original: &Value, answers: &BTreeMap<String, String>) -> Option<Value> {
    let questions = questions(original)?;
    let asked: Vec<&str> = questions.iter().map(|q| q.question.as_str()).collect();
    if !complete(&questions, answers) || answers.keys().any(|k| !asked.contains(&k.as_str())) {
        return None;
    }
    let mut input: Map<String, Value> = original.as_object()?.clone();
    input.insert(
        "answers".into(),
        Value::Object(
            answers
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        ),
    );
    Some(Value::Object(input))
}

/// Feedback for Claude when you send a plan back: your words, trimmed and bounded; `None` if empty.
pub fn feedback(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MAX_FEEDBACK).collect())
}

/// A plan's title: its first heading, else its first line, without Markdown marks.
pub fn plan_title(plan: &str) -> String {
    let lines = || plan.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines()
        .find(|l| l.starts_with('#'))
        .or_else(|| lines().next())
        .unwrap_or("");
    let title = line.trim_start_matches(['#', ' ', '*', '-', '>']).trim();
    let title = title
        .strip_prefix("Plan:")
        .map(str::trim)
        .unwrap_or(title)
        .replace("**", "")
        .replace('`', "");
    super::describe::truncate(&title, MAX_TITLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ask() -> Value {
        json!({ "questions": [
            { "question": "Which framework?", "header": "Framework", "multiSelect": false,
              "options": [{ "label": "React", "description": "Component library" }, { "label": "Vue" }] },
            { "question": "Which extras?", "header": "Extras", "multiSelect": true,
              "options": [{ "label": "Router" }, { "label": "Tests" }, { "label": "Lint" }] },
        ]})
    }

    #[test]
    fn parses_questions() {
        let qs = questions(&ask()).unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].header, "Framework");
        assert_eq!(qs[0].options[1].description, "");
        assert!(qs[1].multi_select);
        assert!(questions(&json!({ "questions": [] })).is_none());
        assert!(questions(&json!({ "questions": [{ "question": "x", "options": [] }] })).is_none());
        assert!(questions(&json!({})).is_none());
    }

    #[test]
    fn answers_are_checked_against_the_options() {
        let qs = questions(&ask()).unwrap();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            answer(&qs, "Which framework?", &s(&["Vue"]), None),
            Some(("Which framework?".into(), "Vue".into()))
        );
        assert_eq!(answer(&qs, "Which framework?", &s(&["Svelte"]), None), None);
        assert_eq!(
            answer(&qs, "Which framework?", &s(&["React", "Vue"]), None),
            None,
            "single select"
        );
        assert_eq!(
            answer(&qs, "Which extras?", &s(&["Lint", "Router"]), None),
            Some(("Which extras?".into(), "Router, Lint".into())),
            "in the options' order, joined by commas"
        );
        assert_eq!(
            answer(&qs, "Which framework?", &[], Some("  Solid, please ")),
            Some(("Which framework?".into(), "Solid, please".into()))
        );
        assert_eq!(answer(&qs, "Which framework?", &[], Some("   ")), None);
        assert_eq!(answer(&qs, "Which framework?", &[], None), None);
        assert_eq!(answer(&qs, "Not asked?", &s(&["Vue"]), None), None);
    }

    #[test]
    fn updated_input_only_when_complete() {
        let mut answers = BTreeMap::new();
        answers.insert("Which framework?".to_string(), "React".to_string());
        assert_eq!(answered_input(&ask(), &answers), None, "one unanswered");
        answers.insert("Which extras?".to_string(), "Tests".to_string());
        let input = answered_input(&ask(), &answers).unwrap();
        assert_eq!(
            input["questions"],
            ask()["questions"],
            "the original questions"
        );
        assert_eq!(input["answers"]["Which framework?"], "React");
        answers.insert("Injected?".to_string(), "x".to_string());
        assert_eq!(answered_input(&ask(), &answers), None, "nothing extra");
        assert_eq!(
            answered_input(&json!({ "plan": "x" }), &BTreeMap::new()),
            None
        );
    }

    #[test]
    fn plans() {
        let plan = "# Plan: create hello.txt\n\n## Context\nCreate a file.\n";
        assert_eq!(plan_title(plan), "create hello.txt");
        assert_eq!(
            plan_title("1. **Refactor** `auth`\n2. Test"),
            "1. Refactor auth"
        );
        let Some(Interaction::Plan { title, lines }) =
            interaction(PLAN_TOOL, &json!({ "plan": plan }), &BTreeMap::new())
        else {
            panic!()
        };
        assert_eq!((title.as_str(), lines), ("create hello.txt", 4));
        assert_eq!(feedback("  "), None);
        assert_eq!(feedback(" add tests ").as_deref(), Some("add tests"));
        assert!(interaction("Bash", &json!({}), &BTreeMap::new()).is_none());
        assert!(is_interactive(QUESTION_TOOL) && !is_interactive("Write"));
    }
}

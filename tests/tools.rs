mod common;
use bridle::events::ToolStatus;
use bridle::tools::{Action, Prepared, definitions, execute, paths, prepare, summarize};
use std::path::Path;

fn run(action: &Action, ctx: &bridle::tools::ToolContext) -> (ToolStatus, String) {
    execute(action, ctx, &mut |_| {}, &mut || false)
}

#[test]
fn definitions_name_the_four_tools() {
    let defs = definitions();
    let names: Vec<&str> =
        defs.as_array().unwrap().iter().map(|d| d["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["read", "bash", "edit", "write"]);
}

#[test]
fn read_needs_no_approval() {
    let ctx = common::context();
    std::fs::write(ctx.cwd.join("a.txt"), "hi\n").unwrap();
    let Prepared::Ready(action) = prepare("read", r#"{"path":"a.txt"}"#, &ctx) else { panic!() };
    assert_eq!(run(&action, &ctx), (ToolStatus::Ok, "     1│ hi\n".to_string()));
}

#[test]
fn edit_previews_a_diff_and_applies_exactly_that() {
    let ctx = common::context();
    let f = ctx.cwd.join("m.rs");
    std::fs::write(&f, "fn main() {\n    let v = false;\n}\n").unwrap();
    let args = r#"{"path":"m.rs","old_text":"let v = false;","new_text":"let v = true;"}"#;
    let Prepared::NeedsApproval(req, action) = prepare("edit", args, &ctx) else { panic!() };
    assert_eq!(req.title, "Approve edit?");
    assert!(req.body.contains("-    let v = false;\n+    let v = true;\n"), "{}", req.body);
    assert!(!req.outside_project);
    assert_eq!(run(&action, &ctx), (ToolStatus::Ok, format!("Edited {} (−1 +1 lines)", f.display())));
    assert!(std::fs::read_to_string(&f).unwrap().contains("let v = true;"));
}

#[test]
fn edit_refuses_if_file_changed_after_preview() {
    let ctx = common::context();
    let f = ctx.cwd.join("m.txt");
    std::fs::write(&f, "a\n").unwrap();
    let Prepared::NeedsApproval(_, action) =
        prepare("edit", r#"{"path":"m.txt","old_text":"a","new_text":"b"}"#, &ctx)
    else {
        panic!()
    };
    std::fs::write(&f, "a\nmore\n").unwrap();
    let (status, msg) = run(&action, &ctx);
    assert_eq!(status, ToolStatus::Error);
    assert!(msg.starts_with("Error: ") && msg.contains("changed since"));
}

#[test]
fn write_new_file_shows_content_and_overwrite_shows_diff() {
    let ctx = common::context();
    let Prepared::NeedsApproval(req, action) =
        prepare("write", r#"{"path":"d/new.txt","content":"x\ny\n"}"#, &ctx)
    else {
        panic!()
    };
    assert_eq!(req.body, "x\ny\n");
    assert!(!req.overwrite);
    assert_eq!(run(&action, &ctx).0, ToolStatus::Ok);

    let Prepared::NeedsApproval(req, _) =
        prepare("write", r#"{"path":"d/new.txt","content":"x\nz\n"}"#, &ctx)
    else {
        panic!()
    };
    assert!(req.overwrite);
    assert!(req.body.contains("-y\n+z\n"));
}

#[test]
fn writes_outside_project_are_flagged() {
    let ctx = common::context();
    let Prepared::NeedsApproval(req, _) =
        prepare("write", r#"{"path":"../elsewhere.txt","content":"x"}"#, &ctx)
    else {
        panic!()
    };
    assert!(req.outside_project);
}

#[test]
fn bash_is_guarded_before_approval() {
    let ctx = common::context();
    let Prepared::Failed(msg) = prepare("bash", r#"{"command":"sudo ls"}"#, &ctx) else { panic!() };
    assert!(msg.contains("privilege escalation"));
    let Prepared::NeedsApproval(req, Action::Bash { command, timeout }) =
        prepare("bash", r#"{"command":"ls -la","timeout":9999}"#, &ctx)
    else {
        panic!()
    };
    assert_eq!(req.body, "ls -la");
    assert_eq!(command, "ls -la");
    assert_eq!(timeout, 600);
}

#[test]
fn bad_arguments_and_unknown_tools_fail_helpfully() {
    let ctx = common::context();
    let Prepared::Failed(msg) = prepare("read", "{not json", &ctx) else { panic!() };
    assert!(msg.contains("not valid JSON") && msg.contains("{not json"));
    let Prepared::Failed(msg) = prepare("read", "{}", &ctx) else { panic!() };
    assert!(msg.contains("missing string argument 'path'"));
    let Prepared::Failed(msg) = prepare("rm", "{}", &ctx) else { panic!() };
    assert!(msg.contains("unknown tool 'rm'"));
}

#[test]
fn summaries() {
    assert_eq!(summarize("bash", r#"{"command":"cargo build\nmore"}"#), "bash  cargo build");
    assert_eq!(summarize("read", r#"{"path":"src/x.rs"}"#), "read  src/x.rs");
    assert_eq!(summarize("read", "garbage"), "read  ");
}

#[test]
fn path_resolution() {
    let cwd = Path::new("/proj/sub");
    assert_eq!(paths::resolve(cwd, "a/../b.txt"), Path::new("/proj/sub/b.txt"));
    assert_eq!(paths::resolve(cwd, "../x"), Path::new("/proj/x"));
    assert_eq!(paths::resolve(cwd, "/etc/hosts"), Path::new("/etc/hosts"));
    let home = std::env::var("HOME").unwrap();
    assert_eq!(paths::resolve(cwd, "~/f"), Path::new(&home).join("f"));
    assert!(paths::is_outside(Path::new("/proj"), Path::new("/projx/a")));
    assert!(!paths::is_outside(Path::new("/proj"), Path::new("/proj/a")));
}

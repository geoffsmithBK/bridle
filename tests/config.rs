use bridle::config::{Config, DEFAULT_URL, Resume, parse};

fn args(s: &[&str]) -> Vec<String> {
    s.iter().map(|a| a.to_string()).collect()
}

#[test]
fn defaults() {
    assert_eq!(
        parse(args(&[])).unwrap(),
        Some(Config { url: DEFAULT_URL.into(), model: None, wire_raw: false, resume: None })
    );
}

#[test]
fn flags() {
    let c = parse(args(&["--url", "http://h:1", "--model", "m", "--wire-raw"])).unwrap().unwrap();
    assert_eq!(c, Config { url: "http://h:1".into(), model: Some("m".into()), wire_raw: true, resume: None });
}

#[test]
fn help_and_errors() {
    assert_eq!(parse(args(&["--help"])).unwrap(), None);
    assert!(parse(args(&["--bogus"])).is_err());
}

#[test]
fn resume_flags() {
    assert_eq!(parse(args(&["--continue"])).unwrap().unwrap().resume, Some(Resume::Latest));
    assert_eq!(parse(args(&["-c"])).unwrap().unwrap().resume, Some(Resume::Latest));
    assert_eq!(
        parse(args(&["--resume", "2026-10-26T09-00-00"])).unwrap().unwrap().resume,
        Some(Resume::Stamp("2026-10-26T09-00-00".into()))
    );
}

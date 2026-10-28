use super::*;

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(args)
}

#[test]
fn dir_positional_parses_path_forms() {
    let cli = parse(&["s7s", "."]).expect("parse");
    assert_eq!(cli.dir.as_deref(), Some("."));
    assert!(cli.command.is_none());

    let cli = parse(&["s7s", "../other-project"]).expect("parse");
    assert_eq!(cli.dir.as_deref(), Some("../other-project"));
}

#[test]
fn subcommand_names_win_over_dir_positional() {
    // A folder literally named like a subcommand is unreachable as a bare word;
    // a path form or `--` is the documented escape hatch.
    assert!(matches!(
        parse(&["s7s", "demo"]).expect("parse").command,
        Some(CliCommand::Demo)
    ));
    assert_eq!(
        parse(&["s7s", "./demo"]).expect("parse").dir.as_deref(),
        Some("./demo")
    );
    assert_eq!(
        parse(&["s7s", "--", "demo"]).expect("parse").dir.as_deref(),
        Some("demo")
    );
}

#[test]
fn dir_with_subcommand_is_rejected() {
    // clap accepts this shape, so the explicit check owns the rejection.
    let cli = parse(&["s7s", ".", "demo"]).expect("parse");
    assert!(cli.dir_conflicts_with_subcommand());
    assert!(!parse(&["s7s", "."])
        .expect("parse")
        .dir_conflicts_with_subcommand());
    assert!(!parse(&["s7s", "demo"])
        .expect("parse")
        .dir_conflicts_with_subcommand());
}

#[test]
fn dir_conflicts_with_debug_only_flags_but_not_rebuild_cache() {
    assert!(parse(&["s7s", "--print", "."]).is_err());
    assert!(parse(&["s7s", "--usage-probe", "."]).is_err());
    assert!(parse(&["s7s", "--model-probe", "."]).is_err());
    // `--handoff-samples` takes an optional value, so a trailing `.` is its
    // output directory, not <DIR>; the conflict shows in the other order.
    assert_eq!(
        parse(&["s7s", "--handoff-samples", "."])
            .expect("parse")
            .handoff_samples,
        Some(Some(std::path::PathBuf::from(".")))
    );
    assert!(parse(&["s7s", ".", "--handoff-samples"]).is_err());
    // Rebuilding the index before the dialog opens is a valid combination.
    let cli = parse(&["s7s", "--rebuild-cache", "."]).expect("parse");
    assert!(cli.rebuild_cache);
    assert_eq!(cli.dir.as_deref(), Some("."));
}

#[test]
fn resolve_startup_dir_accepts_existing_relative_dir() {
    // Tests run with the crate root as cwd.
    let resolved = resolve_startup_dir("src").expect("resolve");
    assert!(resolved.is_absolute());
    assert!(resolved.is_dir());
    assert!(resolved.ends_with("src"));
    let dot = resolve_startup_dir(".").expect("resolve");
    assert_eq!(dot, std::fs::canonicalize(".").expect("canonicalize"));
}

#[test]
fn resolve_startup_dir_rejects_missing_path_file_and_empty() {
    assert!(resolve_startup_dir("no-such-folder-xyz").is_err());
    assert!(resolve_startup_dir("Cargo.toml")
        .expect_err("file must be rejected")
        .contains("not a directory"));
    assert!(resolve_startup_dir("   ").is_err());
}

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "lint-docs".to_string());
    let rest = args.collect::<Vec<_>>();
    match command.as_str() {
        "lint-docs" => lint_docs(&rest),
        "report" => report(&rest),
        _ => {
            eprintln!(
                "usage: ratatoskr-coverage [lint-docs [DOCS_ROOT] | report [ROOT] [--area ID_PREFIX] [--strict]]"
            );
            ExitCode::FAILURE
        }
    }
}

fn lint_docs(args: &[String]) -> ExitCode {
    let Some(docs_root) = optional_path_arg(args, "lint-docs", PathBuf::from("docs")) else {
        return ExitCode::FAILURE;
    };
    let catalog = coverage::load_docs(&docs_root);
    for diagnostic in &catalog.diagnostics {
        if diagnostic.line == 0 {
            eprintln!("{}: {}", diagnostic.file.display(), diagnostic.message);
        } else {
            eprintln!(
                "{}:{}: {}",
                diagnostic.file.display(),
                diagnostic.line,
                diagnostic.message
            );
        }
    }

    if !catalog.is_clean() {
        eprintln!(
            "coverage doc lint failed: {} contract markers, {} diagnostics",
            catalog.contracts.len(),
            catalog.diagnostics.len()
        );
        return ExitCode::FAILURE;
    }

    println!(
        "coverage doc lint passed: {} contract markers",
        catalog.contracts.len()
    );
    ExitCode::SUCCESS
}

fn report(args: &[String]) -> ExitCode {
    let Some(args) = parse_report_args(args) else {
        return ExitCode::FAILURE;
    };
    let root = args.root;
    let docs_root = root.join("docs");
    let lua_roots = vec![
        root.join("crates/app/tests/service-harness"),
        root.join("crates/app/tests/sync-harness"),
    ];
    let report = coverage::CoverageReport::build(&docs_root, &lua_roots);
    let area = args.area.as_deref();

    let contracts = report
        .catalog
        .contracts
        .iter()
        .filter(|contract| matches_area(&contract.id, area))
        .collect::<Vec<_>>();
    let uncovered_contracts = report
        .uncovered_contracts
        .iter()
        .filter(|contract| matches_area(&contract.id, area))
        .collect::<Vec<_>>();
    let unknown_lua_claims = report
        .unknown_lua_claims
        .iter()
        .filter(|claim| matches_area(&claim.id, area))
        .collect::<Vec<_>>();

    if let Some(area) = area {
        println!("area: {area}");
    }
    println!("registered contracts: {}", contracts.len());
    for contract in contracts {
        let enforcement = contract
            .enforcement
            .map(coverage::Enforcement::as_str)
            .unwrap_or("unspecified");
        println!(
            "{} {}:{} status={} enforcement={} section=\"{}\"",
            contract.id,
            contract.file.display(),
            contract.line,
            contract.status.as_str(),
            enforcement,
            contract.section.title
        );
    }

    if !report.catalog.diagnostics.is_empty() {
        println!();
        println!("doc diagnostics: {}", report.catalog.diagnostics.len());
        print_diagnostics(&report.catalog.diagnostics);
    }
    if !report.lua_claims.diagnostics.is_empty() {
        println!();
        println!("lua claim diagnostics: {}", report.lua_claims.diagnostics.len());
        print_diagnostics(&report.lua_claims.diagnostics);
    }

    println!();
    println!(
        "registered contracts with no Lua claim: {}",
        uncovered_contracts.len()
    );
    for contract in &uncovered_contracts {
        println!("{} {}:{}", contract.id, contract.file.display(), contract.line);
    }

    println!();
    if area.is_some() {
        println!("Lua tests with no covers claim: skipped by area filter");
    } else {
        println!(
            "Lua tests with no covers claim: {}",
            report.lua_tests_without_claims.len()
        );
        for file in &report.lua_tests_without_claims {
            println!("{}", file.display());
        }
    }

    println!();
    println!("unknown Lua claims: {}", unknown_lua_claims.len());
    for claim in &unknown_lua_claims {
        println!("{}:{} {}", claim.file.display(), claim.line, claim.id);
    }

    if args.strict {
        let strict_failures = report.catalog.diagnostics.len()
            + strict_lua_diagnostic_count(&report)
            + uncovered_contracts.len()
            + strict_no_claim_count(&report, area)
            + unknown_lua_claims.len();
        if strict_failures > 0 {
            eprintln!("coverage strict report failed: {strict_failures} findings");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

struct ReportArgs {
    root: PathBuf,
    area: Option<String>,
    strict: bool,
}

fn parse_report_args(args: &[String]) -> Option<ReportArgs> {
    let mut root = None;
    let mut area = None;
    let mut strict = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--area" {
            index += 1;
            let Some(value) = args.get(index) else {
                eprintln!(
                    "usage: ratatoskr-coverage report [ROOT] [--area ID_PREFIX] [--strict]"
                );
                return None;
            };
            area = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--area=") {
            area = Some(value.to_string());
        } else if arg == "--strict" {
            strict = true;
        } else if root.is_none() {
            root = Some(PathBuf::from(arg));
        } else {
            eprintln!("usage: ratatoskr-coverage report [ROOT] [--area ID_PREFIX] [--strict]");
            return None;
        }
        index += 1;
    }

    Some(ReportArgs {
        root: root.unwrap_or_else(|| PathBuf::from(".")),
        area,
        strict,
    })
}

fn matches_area(id: &str, area: Option<&str>) -> bool {
    let Some(prefix) = area else {
        return true;
    };
    id == prefix
        || id
            .strip_prefix(prefix)
            .map(|rest| rest.starts_with('.'))
            .unwrap_or(false)
}

fn optional_path_arg(args: &[String], command: &str, default: PathBuf) -> Option<PathBuf> {
    if args.len() > 1 {
        eprintln!("usage: ratatoskr-coverage {command} [PATH]");
        return None;
    }
    Some(
        args.iter()
            .next()
            .map(PathBuf::from)
            .unwrap_or(default),
    )
}

fn strict_lua_diagnostic_count(report: &coverage::CoverageReport) -> usize {
    report.lua_claims.diagnostics.len()
}

fn strict_no_claim_count(report: &coverage::CoverageReport, area: Option<&str>) -> usize {
    if area.is_some() {
        0
    } else {
        report.lua_tests_without_claims.len()
    }
}

fn print_diagnostics(diagnostics: &[coverage::Diagnostic]) {
    for diagnostic in diagnostics {
        if diagnostic.line == 0 {
            println!("{}: {}", diagnostic.file.display(), diagnostic.message);
        } else {
            println!(
                "{}:{}: {}",
                diagnostic.file.display(),
                diagnostic.line,
                diagnostic.message
            );
        }
    }
}

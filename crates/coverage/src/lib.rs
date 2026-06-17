use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MARKER_PREFIX: &str = "<!-- coverage:";
const MARKER_SUFFIX: &str = "-->";
const LUA_COVERS_PREFIX: &str = "-- @covers:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub contracts: Vec<Contract>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Catalog {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub id: String,
    pub file: PathBuf,
    pub line: usize,
    pub section: Section,
    pub status: ContractStatus,
    pub enforcement: Option<Enforcement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub level: usize,
    pub title: String,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractStatus {
    Required,
    Gap,
    Retired,
    CompileEnforced,
}

impl ContractStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "required" => Some(Self::Required),
            "gap" => Some(Self::Gap),
            "retired" => Some(Self::Retired),
            "compile-enforced" => Some(Self::CompileEnforced),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Gap => "gap",
            Self::Retired => "retired",
            Self::CompileEnforced => "compile-enforced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enforcement {
    Compiler,
    RustTest,
    LuaHarness,
    Convention,
    Mixed,
}

impl Enforcement {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "compiler" => Some(Self::Compiler),
            "rust-test" => Some(Self::RustTest),
            "lua-harness" => Some(Self::LuaHarness),
            "convention" => Some(Self::Convention),
            "mixed" => Some(Self::Mixed),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compiler => "compiler",
            Self::RustTest => "rust-test",
            Self::LuaHarness => "lua-harness",
            Self::Convention => "convention",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub kind: DiagnosticKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    DuplicateId,
    MalformedMarker,
    InvalidId,
    InvalidMetadata,
    OrphanedMarker,
    MisplacedMarker,
    Io,
}

#[must_use]
pub fn parse_markdown(file: impl Into<PathBuf>, source: &str) -> Catalog {
    let file = file.into();
    let mut contracts = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current_section = None;
    let mut marker_allowed = false;
    let mut in_fenced_code = false;

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed_start = line.trim_start();
        if is_fence(trimmed_start) {
            in_fenced_code = !in_fenced_code;
            marker_allowed = false;
            continue;
        }
        if in_fenced_code {
            continue;
        }

        if let Some(section) = parse_heading(line, line_number) {
            current_section = Some(section);
            marker_allowed = true;
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() && marker_allowed {
            continue;
        }
        if !trimmed.starts_with(MARKER_PREFIX) {
            marker_allowed = false;
            continue;
        }

        let Some(section) = current_section.clone() else {
            diagnostics.push(Diagnostic {
                file: file.clone(),
                line: line_number,
                kind: DiagnosticKind::OrphanedMarker,
                message: "coverage marker has no preceding markdown section".to_string(),
            });
            continue;
        };
        if !marker_allowed {
            diagnostics.push(Diagnostic {
                file: file.clone(),
                line: line_number,
                kind: DiagnosticKind::MisplacedMarker,
                message: "coverage marker must be placed immediately after its heading".to_string(),
            });
            continue;
        }

        match parse_marker(trimmed) {
            Ok(marker) => {
                contracts.push(Contract {
                    id: marker.id,
                    file: file.clone(),
                    line: line_number,
                    section,
                    status: marker.status,
                    enforcement: marker.enforcement,
                });
            }
            Err(message) => diagnostics.push(Diagnostic {
                file: file.clone(),
                line: line_number,
                kind: diagnostic_kind_for_marker_error(&message),
                message,
            }),
        }
    }

    Catalog {
        contracts,
        diagnostics,
    }
}

pub fn load_docs(root: impl AsRef<Path>) -> Catalog {
    let root = root.as_ref();
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    collect_markdown_files(root, &mut files, &mut diagnostics);
    files.sort();

    let mut contracts = Vec::new();
    for file in files {
        match fs::read_to_string(&file) {
            Ok(source) => {
                let parsed = parse_markdown(file, &source);
                contracts.extend(parsed.contracts);
                diagnostics.extend(parsed.diagnostics);
            }
            Err(error) => diagnostics.push(Diagnostic {
                file,
                line: 0,
                kind: DiagnosticKind::Io,
                message: format!("failed to read markdown file: {error}"),
            }),
        }
    }

    diagnostics.extend(duplicate_diagnostics(&contracts));
    contracts.sort_by(|left, right| left.id.cmp(&right.id));

    Catalog {
        contracts,
        diagnostics,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaClaimCatalog {
    pub tests: Vec<LuaTestClaims>,
    pub diagnostics: Vec<Diagnostic>,
}

impl LuaClaimCatalog {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaTestClaims {
    pub file: PathBuf,
    pub claims: Vec<LuaClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaClaim {
    pub id: String,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    pub catalog: Catalog,
    pub lua_claims: LuaClaimCatalog,
    pub uncovered_contracts: Vec<Contract>,
    pub lua_tests_without_claims: Vec<PathBuf>,
    pub unknown_lua_claims: Vec<LuaClaim>,
}

impl CoverageReport {
    #[must_use]
    pub fn build(docs_root: impl AsRef<Path>, lua_roots: &[PathBuf]) -> Self {
        let catalog = load_docs(docs_root);
        let lua_claims = load_lua_claims(lua_roots);
        build_report(catalog, lua_claims)
    }
}

#[must_use]
pub fn parse_lua_claims(file: impl Into<PathBuf>, source: &str) -> LuaClaimCatalog {
    let file = file.into();
    let mut claims = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen_frontmatter = false;

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            seen_frontmatter = true;
            if let Some(rest) = trimmed.strip_prefix(LUA_COVERS_PREFIX) {
                match parse_covers_value(rest.trim()) {
                    Ok(id) => claims.push(LuaClaim {
                        id: id.to_string(),
                        file: file.clone(),
                        line: line_number,
                    }),
                    Err(message) => diagnostics.push(Diagnostic {
                        file: file.clone(),
                        line: line_number,
                        kind: diagnostic_kind_for_lua_claim_error(&message),
                        message,
                    }),
                }
            }
            continue;
        }
        if trimmed.is_empty() && !seen_frontmatter {
            continue;
        }
        break;
    }

    LuaClaimCatalog {
        tests: vec![LuaTestClaims { file, claims }],
        diagnostics,
    }
}

#[must_use]
pub fn load_lua_claims(roots: &[PathBuf]) -> LuaClaimCatalog {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for root in roots {
        collect_files_with_extension(root, "lua", &mut files, &mut diagnostics);
    }
    files.sort();

    let mut tests = Vec::new();
    for file in files {
        match fs::read_to_string(&file) {
            Ok(source) => {
                let parsed = parse_lua_claims(file, &source);
                tests.extend(parsed.tests);
                diagnostics.extend(parsed.diagnostics);
            }
            Err(error) => diagnostics.push(Diagnostic {
                file,
                line: 0,
                kind: DiagnosticKind::Io,
                message: format!("failed to read Lua harness script: {error}"),
            }),
        }
    }

    LuaClaimCatalog { tests, diagnostics }
}

#[must_use]
pub fn build_report(catalog: Catalog, lua_claims: LuaClaimCatalog) -> CoverageReport {
    let registered_ids = catalog
        .contracts
        .iter()
        .map(|contract| contract.id.as_str())
        .collect::<HashSet<_>>();
    let claimed_ids = lua_claims
        .tests
        .iter()
        .flat_map(|test| test.claims.iter().map(|claim| claim.id.as_str()))
        .collect::<HashSet<_>>();

    let uncovered_contracts = catalog
        .contracts
        .iter()
        .filter(|contract| contract_requires_test_claim(contract))
        .filter(|contract| !claimed_ids.contains(contract.id.as_str()))
        .cloned()
        .collect();
    let lua_tests_without_claims = lua_claims
        .tests
        .iter()
        .filter(|test| test.claims.is_empty())
        .map(|test| test.file.clone())
        .collect();
    let unknown_lua_claims = lua_claims
        .tests
        .iter()
        .flat_map(|test| {
            test.claims
                .iter()
                .filter(|claim| !registered_ids.contains(claim.id.as_str()))
                .cloned()
        })
        .collect();

    CoverageReport {
        catalog,
        lua_claims,
        uncovered_contracts,
        lua_tests_without_claims,
        unknown_lua_claims,
    }
}

#[must_use]
pub fn is_valid_contract_id(id: &str) -> bool {
    let mut segments = id.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    if !is_valid_id_segment(first) {
        return false;
    }

    let mut segment_count = 1;
    for segment in segments {
        segment_count += 1;
        if !is_valid_id_segment(segment) {
            return false;
        }
    }
    segment_count >= 2
}

fn is_valid_id_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn parse_heading(line: &str, line_number: usize) -> Option<Section> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let title = trimmed.get(level..)?;
    if !title.starts_with(' ') {
        return None;
    }
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    Some(Section {
        level,
        title: title.to_string(),
        line: line_number,
    })
}

fn is_fence(trimmed_start: &str) -> bool {
    trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    id: String,
    status: ContractStatus,
    enforcement: Option<Enforcement>,
}

fn parse_marker(line: &str) -> Result<Marker, String> {
    if !line.ends_with(MARKER_SUFFIX) {
        return Err("coverage marker must end with `-->`".to_string());
    }
    let body = line
        .strip_prefix(MARKER_PREFIX)
        .and_then(|value| value.strip_suffix(MARKER_SUFFIX))
        .map(str::trim)
        .unwrap_or_default();
    let mut parts = body.split_whitespace();
    let Some(id) = parts.next() else {
        return Err("coverage marker is missing a contract id".to_string());
    };
    if !is_valid_contract_id(id) {
        return Err(format!("invalid coverage contract id `{id}`"));
    }

    let mut status = ContractStatus::Required;
    let mut enforcement = None;
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!("invalid coverage marker metadata `{part}`"));
        };
        match key {
            "status" => {
                status = ContractStatus::parse(value)
                    .ok_or_else(|| format!("invalid coverage marker status `{value}`"))?;
            }
            "enforcement" => {
                enforcement = Some(
                    Enforcement::parse(value)
                        .ok_or_else(|| format!("invalid coverage enforcement `{value}`"))?,
                );
            }
            _ => return Err(format!("unknown coverage marker metadata key `{key}`")),
        }
    }

    Ok(Marker {
        id: id.to_string(),
        status,
        enforcement,
    })
}

fn diagnostic_kind_for_marker_error(message: &str) -> DiagnosticKind {
    if message.starts_with("invalid coverage contract id") {
        DiagnosticKind::InvalidId
    } else if message.contains("metadata")
        || message.contains("status")
        || message.contains("enforcement")
    {
        DiagnosticKind::InvalidMetadata
    } else {
        DiagnosticKind::MalformedMarker
    }
}

fn collect_markdown_files(
    root: &Path,
    files: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    collect_files_with_extension(root, "md", files, diagnostics);
}

fn collect_files_with_extension(
    root: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(Diagnostic {
                file: root.to_path_buf(),
                line: 0,
                kind: DiagnosticKind::Io,
                message: format!("failed to read directory: {error}"),
            });
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    file: root.to_path_buf(),
                    line: 0,
                    kind: DiagnosticKind::Io,
                    message: format!("failed to read directory entry: {error}"),
                });
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, files, diagnostics);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            files.push(path);
        }
    }
}

fn parse_covers_value(value: &str) -> Result<&str, String> {
    if value.is_empty() {
        return Err("Lua coverage claim is missing a contract id".to_string());
    }
    if value.contains(',') || value.split_whitespace().count() != 1 {
        return Err(format!(
            "Lua coverage claim `{value}` must name exactly one contract id"
        ));
    }
    if !is_valid_contract_id(value) {
        return Err(format!("invalid coverage contract id `{value}`"));
    }
    Ok(value)
}

fn diagnostic_kind_for_lua_claim_error(message: &str) -> DiagnosticKind {
    if message.starts_with("invalid coverage contract id") || message.contains("missing") {
        DiagnosticKind::InvalidId
    } else {
        DiagnosticKind::InvalidMetadata
    }
}

fn contract_requires_test_claim(contract: &Contract) -> bool {
    match contract.status {
        ContractStatus::Gap | ContractStatus::Retired | ContractStatus::CompileEnforced => false,
        ContractStatus::Required => contract.enforcement != Some(Enforcement::Compiler),
    }
}

fn duplicate_diagnostics(contracts: &[Contract]) -> Vec<Diagnostic> {
    let mut first_seen: HashMap<String, &Contract> = HashMap::new();
    let mut diagnostics = Vec::new();
    for contract in contracts {
        if let Some(first) = first_seen.get(&contract.id) {
            diagnostics.push(Diagnostic {
                file: contract.file.clone(),
                line: contract.line,
                kind: DiagnosticKind::DuplicateId,
                message: format!(
                    "duplicate coverage contract id `{}` first declared at {}:{}",
                    contract.id,
                    first.file.display(),
                    first.line
                ),
            });
        } else {
            first_seen.insert(contract.id.clone(), contract);
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marker_below_heading() {
        let parsed = parse_markdown(
            "docs/example.md",
            "# Example\n\n## Contract\n<!-- coverage: example.contract enforcement=compiler -->\nText\n",
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.contracts.len(), 1);
        let contract = &parsed.contracts[0];
        assert_eq!(contract.id, "example.contract");
        assert_eq!(contract.section.title, "Contract");
        assert_eq!(contract.section.level, 2);
        assert_eq!(contract.status, ContractStatus::Required);
        assert_eq!(contract.enforcement, Some(Enforcement::Compiler));
    }

    #[test]
    fn parses_status_metadata() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\n<!-- coverage: example.contract status=gap enforcement=lua-harness -->\n",
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.contracts[0].status, ContractStatus::Gap);
        assert_eq!(
            parsed.contracts[0].enforcement,
            Some(Enforcement::LuaHarness)
        );
    }

    #[test]
    fn rejects_orphaned_marker() {
        let parsed = parse_markdown(
            "docs/example.md",
            "<!-- coverage: example.contract -->\n## Contract\n",
        );

        assert!(parsed.contracts.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, DiagnosticKind::OrphanedMarker);
    }

    #[test]
    fn rejects_marker_after_section_body() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\nText\n<!-- coverage: example.contract -->\n",
        );

        assert!(parsed.contracts.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, DiagnosticKind::MisplacedMarker);
    }

    #[test]
    fn allows_blank_lines_between_heading_and_marker() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\n\n<!-- coverage: example.contract -->\nText\n",
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.contracts.len(), 1);
        assert_eq!(parsed.contracts[0].id, "example.contract");
    }

    #[test]
    fn rejects_invalid_id() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\n<!-- coverage: Example.contract -->\n",
        );

        assert!(parsed.contracts.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, DiagnosticKind::InvalidId);
    }

    #[test]
    fn rejects_invalid_metadata() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\n<!-- coverage: example.contract status=later -->\n",
        );

        assert!(parsed.contracts.is_empty());
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, DiagnosticKind::InvalidMetadata);
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut first = parse_markdown(
            "docs/first.md",
            "## First\n<!-- coverage: example.contract -->\n",
        );
        let second = parse_markdown(
            "docs/second.md",
            "## Second\n<!-- coverage: example.contract -->\n",
        );
        first.contracts.extend(second.contracts);

        let diagnostics = duplicate_diagnostics(&first.contracts);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::DuplicateId);
        assert_eq!(diagnostics[0].file, PathBuf::from("docs/second.md"));
    }

    #[test]
    fn ignores_markers_inside_fenced_code() {
        let parsed = parse_markdown(
            "docs/example.md",
            "## Contract\n```markdown\n<!-- coverage: example.contract -->\n```\n",
        );

        assert!(parsed.contracts.is_empty());
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn parses_lua_frontmatter_claims() {
        let parsed = parse_lua_claims(
            "test.lua",
            "-- description: sample\n-- @covers: architecture.action_service_as_mutation_gate\n\nprint('body')\n",
        );

        assert!(parsed.diagnostics.is_empty());
        assert_eq!(parsed.tests.len(), 1);
        assert_eq!(parsed.tests[0].claims.len(), 1);
        assert_eq!(
            parsed.tests[0].claims[0].id,
            "architecture.action_service_as_mutation_gate"
        );
    }

    #[test]
    fn rejects_multi_id_lua_claim_line() {
        let parsed = parse_lua_claims(
            "test.lua",
            "-- @covers: architecture.one, architecture.two\n",
        );

        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].kind, DiagnosticKind::InvalidMetadata);
    }

    #[test]
    fn ignores_lua_claims_outside_frontmatter() {
        let parsed = parse_lua_claims(
            "test.lua",
            "-- description: sample\n\n-- @covers: architecture.action_service_as_mutation_gate\n",
        );

        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.tests[0].claims.is_empty());
    }

    #[test]
    fn report_finds_uncovered_and_unknown_lua_claims() {
        let catalog = parse_markdown(
            "docs/example.md",
            "## One\n<!-- coverage: architecture.one enforcement=lua-harness -->\n\
             ## Two\n<!-- coverage: architecture.two enforcement=compiler -->\n",
        );
        let lua_claims = parse_lua_claims("test.lua", "-- @covers: architecture.missing\n");

        let report = build_report(catalog, lua_claims);

        assert_eq!(report.uncovered_contracts.len(), 1);
        assert_eq!(report.uncovered_contracts[0].id, "architecture.one");
        assert_eq!(report.unknown_lua_claims.len(), 1);
        assert_eq!(report.unknown_lua_claims[0].id, "architecture.missing");
    }

    #[test]
    fn validates_contract_id_grammar() {
        assert!(is_valid_contract_id(
            "architecture.action_service_as_mutation_gate"
        ));
        assert!(is_valid_contract_id(
            "cmdk.requirements.fuzzy_search_word_boundary_weighting"
        ));
        assert!(!is_valid_contract_id("architecture"));
        assert!(!is_valid_contract_id("Architecture.action"));
        assert!(!is_valid_contract_id("architecture.action-service"));
        assert!(!is_valid_contract_id("architecture.1_action"));
        assert!(!is_valid_contract_id("architecture..action"));
    }
}

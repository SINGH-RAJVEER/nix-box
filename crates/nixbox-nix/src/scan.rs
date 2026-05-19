//! Detects packages declared in the user's main nix config (outside the
//! nixbox-managed file) so they can be surfaced in the TUI and optionally
//! migrated into nixbox's manifest.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTarget {
    HomeManager,
    Nixos,
}

/// A package found declared inline in the user's main nix config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPackage {
    /// Display/manifest name (after stripping any `pkgs.` prefix).
    pub name: String,
    /// Source attribute path the package was found in (e.g.
    /// `environment.systemPackages`, `fonts.packages`).
    pub source_attr: String,
    /// 0-indexed line in the source file.
    pub line: usize,
    /// True if this entry is on a dedicated line and can be cleanly removed
    /// from the source as part of a migration. Same-line list entries
    /// (`[ pkgs.foo pkgs.bar ]`) are tagged false.
    pub migratable: bool,
    /// Which target's scope this package belongs to (set by `scan`).
    pub scope: ScanTarget,
}

/// Reads `path` and returns package declarations found inside attribute lists
/// like `home.packages`, `environment.systemPackages`, `fonts.packages`,
/// `xdg.portal.extraPortals`, etc.
pub fn scan(path: &Path, target: ScanTarget) -> Result<Vec<ExternalPackage>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(parse(&raw, target))
}

fn parse(raw: &str, target: ScanTarget) -> Vec<ExternalPackage> {
    let lines: Vec<&str> = raw.lines().collect();
    let mut out = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let stripped = strip_comment(lines[i]);
        let Some(open) = detect_package_list_open(stripped, target) else {
            i += 1;
            continue;
        };

        // Capture entries that appear on the same line as the opener,
        // between `[` and the (optional) matching `]`.
        for name in same_line_entries(stripped, open.with_pkgs) {
            out.push(ExternalPackage {
                name,
                source_attr: open.source_attr.clone(),
                line: i,
                migratable: false,
                scope: target,
            });
        }

        let mut depth = bracket_delta(stripped);
        if depth <= 0 {
            // Whole list closed on this line; nothing more to do.
            i += 1;
            continue;
        }

        let outer_depth = depth;
        let mut j = i + 1;
        while j < lines.len() && depth > 0 {
            let content = strip_comment(lines[j]);
            let delta = bracket_delta(content);
            if depth + delta <= 0 {
                break;
            }
            // Only treat lines at the outermost depth, with no bracket change,
            // as candidate package entries. Lines that open/close nested
            // structures are skipped.
            if depth == outer_depth && delta == 0 {
                if let Some(name) = extract_entry(content, open.with_pkgs) {
                    out.push(ExternalPackage {
                        name,
                        source_attr: open.source_attr.clone(),
                        line: j,
                        migratable: true,
                        scope: target,
                    });
                }
            }
            depth += delta;
            j += 1;
        }
        i = j + 1;
    }

    out
}

#[derive(Debug, Clone)]
struct OpenInfo {
    with_pkgs: bool,
    source_attr: String,
}

/// If `line` opens a package list, returns metadata about the opener.
/// Recognises:
///   * `<key> = [...]` where the last segment of `<key>` looks package-like
///     (`Packages`, `packages`, `Portals`, `portals`, `Themes`, `themes`)
///   * `<key> = with pkgs; [...]` — the `with pkgs;` makes any list a pkg list
fn detect_package_list_open(line: &str, target: ScanTarget) -> Option<OpenInfo> {
    let trimmed = line.trim_start();
    if !trimmed.contains('[') {
        return None;
    }
    let with_pkgs = trimmed.contains("with pkgs;");

    let eq_pos = trimmed.find('=')?;
    // Reject `==`, `=>` and `:=`-ish tokens.
    let bytes = trimmed.as_bytes();
    if bytes.get(eq_pos + 1) == Some(&b'=') || bytes.get(eq_pos + 1) == Some(&b'>') {
        return None;
    }
    // The attribute path is the longest trailing run of ident chars / dots
    // before `=`. Anything before that (a `{` opening an attrset, `let`
    // bindings, etc.) is ignored.
    let lhs = extract_trailing_attr_path(&trimmed[..eq_pos])?;

    let key_like = matches_target(&lhs, target);
    if !key_like && !with_pkgs {
        return None;
    }
    if !key_like && with_pkgs {
        // `with pkgs;` is a strong signal that this is a package list, even
        // when the LHS key doesn't match our suffix heuristic (e.g. a
        // `let myPkgs = with pkgs; [ ... ]` binding). Still respect the target
        // scope: for HM we only want `home.*` keys, for NixOS we exclude them.
        let lhs_in_target_scope = match target {
            ScanTarget::HomeManager => lhs.starts_with("home.") || lhs == "packages",
            ScanTarget::Nixos => !lhs.starts_with("home."),
        };
        if !lhs_in_target_scope {
            return None;
        }
    }

    Some(OpenInfo {
        with_pkgs,
        source_attr: lhs,
    })
}

/// Returns the attribute path that immediately precedes `=`, ignoring any
/// surrounding structure like `{ ` or `let `. Returns `None` if no valid
/// attribute path is found.
fn extract_trailing_attr_path(s: &str) -> Option<String> {
    let chars: Vec<char> = s.trim_end().chars().collect();
    let end = chars.len();
    let mut start = end;
    while start > 0 {
        let c = chars[start - 1];
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '\'' || c == '.' {
            start -= 1;
        } else {
            break;
        }
    }
    let raw: String = chars[start..end].iter().collect();
    let cleaned = raw.trim_matches('.');
    if cleaned.is_empty() {
        return None;
    }
    for seg in cleaned.split('.') {
        if !is_nix_ident(seg) {
            return None;
        }
    }
    Some(cleaned.to_string())
}

/// Returns true if the last `.`-separated segment of `lhs` ends in a
/// package-list-ish suffix and the path is in scope for `target`.
fn matches_target(lhs: &str, target: ScanTarget) -> bool {
    let last = lhs.rsplit('.').next().unwrap_or("");
    let pkg_like = last.ends_with("ackages")
        || last.ends_with("ortals")
        || last.ends_with("hemes");
    if !pkg_like {
        return false;
    }
    // Reject negative lists (`excludePackages`, `disabledPackages`, etc.) —
    // those are packages the user is removing, not declaring.
    let last_lower = last.to_ascii_lowercase();
    if last_lower.contains("exclude") || last_lower.contains("disable") {
        return false;
    }
    match target {
        ScanTarget::HomeManager => lhs.starts_with("home.") || lhs == "packages",
        ScanTarget::Nixos => !lhs.starts_with("home."),
    }
}

/// Pulls entries from text between `[` (on this line) and the next `]` or end
/// of line. Used for inline one-liner lists.
fn same_line_entries(line: &str, with_pkgs: bool) -> Vec<String> {
    let Some(open_pos) = line.find('[') else { return Vec::new() };
    let after_open = &line[open_pos + 1..];
    let inside = match after_open.find(']') {
        Some(p) => &after_open[..p],
        None => after_open,
    };
    inside
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| matches!(c, ';' | ',')))
        .filter_map(|t| clean_token(t, with_pkgs))
        .collect()
}

/// Extracts a single package entry from `line` — must be one token with no
/// surrounding whitespace-separated junk.
fn extract_entry(line: &str, with_pkgs: bool) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    let trimmed = trimmed.trim_matches(|c: char| matches!(c, ';' | ','));
    clean_token(trimmed, with_pkgs)
}

/// Validates and normalises one package token. Returns the manifest-style
/// name (with the `pkgs.` prefix stripped) on success.
fn clean_token(token: &str, with_pkgs: bool) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    // Reject anything with syntactic noise — these are complex expressions,
    // function applications, attribute selections through interpolation,
    // string literals, etc.
    if token.chars().any(|c| matches!(c, '(' | ')' | '{' | '}' | '[' | ']' | '"' | '\'' | '=' | '$' | '\\')) {
        return None;
    }
    let (candidate, had_prefix) = if let Some(rest) = token.strip_prefix("pkgs.") {
        (rest, true)
    } else {
        (token, false)
    };
    if candidate.is_empty() {
        return None;
    }
    // Without an explicit `pkgs.` prefix, we can only trust the entry if it
    // came from a `with pkgs;` block. Otherwise we'd misinterpret things like
    // a bare `inputs.foo` as a package.
    if !had_prefix && !with_pkgs {
        return None;
    }
    // Reject things that are clearly not in pkgs.
    if candidate.starts_with("inputs.") || candidate.starts_with("self.") {
        return None;
    }
    // Each dotted segment must be a valid nix identifier.
    for part in candidate.split('.') {
        if !is_nix_ident(part) {
            return None;
        }
    }
    Some(candidate.to_string())
}

fn is_nix_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '\'')
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(p) => &line[..p],
        None => line,
    }
}

fn bracket_delta(line: &str) -> i32 {
    let mut d: i32 = 0;
    let mut in_string = false;
    let mut prev_backslash = false;
    for c in line.chars() {
        if prev_backslash {
            prev_backslash = false;
            continue;
        }
        match c {
            '\\' if in_string => {
                prev_backslash = true;
            }
            '"' => in_string = !in_string,
            '[' if !in_string => d += 1,
            ']' if !in_string => d -= 1,
            _ => {}
        }
    }
    d
}

/// Rewrites `path` removing every line that the scanner currently associates
/// with one of the package names in `names`. Lines outside package blocks and
/// non-matching package lines are preserved verbatim.
///
/// Only entries flagged `migratable` are eligible — same-line inline entries
/// stay where they are.
///
/// Returns the names that were actually removed.
pub fn remove_from_source(
    path: &Path,
    target: ScanTarget,
    names: &[String],
) -> Result<Vec<String>> {
    if names.is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let externals = parse(&raw, target);
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();

    let mut remove_lines: HashSet<usize> = HashSet::new();
    let mut removed: Vec<String> = Vec::new();
    for ep in externals {
        if ep.migratable && wanted.contains(ep.name.as_str()) {
            remove_lines.insert(ep.line);
            removed.push(ep.name);
        }
    }
    if remove_lines.is_empty() {
        return Ok(Vec::new());
    }

    let trailing_newline = raw.ends_with('\n');
    let mut out = String::with_capacity(raw.len());
    for (idx, line) in raw.lines().enumerate() {
        if remove_lines.contains(&idx) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !trailing_newline && out.ends_with('\n') {
        out.pop();
    }
    fs::write(path, out)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(pkgs: &[ExternalPackage]) -> Vec<&str> {
        pkgs.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn parses_with_pkgs_multiline() {
        let src = r#"
{ pkgs, ... }:
{
  home.packages = with pkgs; [
    ripgrep
    fd
    jq
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::HomeManager);
        assert_eq!(names(&pkgs), vec!["ripgrep", "fd", "jq"]);
        assert!(pkgs.iter().all(|p| p.migratable));
    }

    #[test]
    fn parses_pkgs_prefix() {
        let src = r#"
{
  environment.systemPackages = [
    pkgs.htop
    pkgs.curl
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["htop", "curl"]);
    }

    #[test]
    fn skips_complex_expressions() {
        let src = r#"
{
  home.packages = with pkgs; [
    ripgrep
    (python3.withPackages (ps: [ ps.requests ]))
    fd
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::HomeManager);
        assert_eq!(names(&pkgs), vec!["ripgrep", "fd"]);
    }

    #[test]
    fn allows_dotted_package_paths() {
        let src = r#"
{
  fonts.packages = with pkgs; [
    nerd-fonts._3270
  ];
  hardware.graphics.extraPackages = [
    pkgs.rocmPackages.clr.icd
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"nerd-fonts._3270"), "got {:?}", names);
        assert!(names.contains(&"rocmPackages.clr.icd"), "got {:?}", names);
    }

    #[test]
    fn recognises_portals_attribute() {
        let src = r#"
{
  xdg.portal.extraPortals = [
    pkgs.xdg-desktop-portal-gtk
    pkgs.xdg-desktop-portal-gnome
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(
            names(&pkgs),
            vec!["xdg-desktop-portal-gtk", "xdg-desktop-portal-gnome"]
        );
    }

    #[test]
    fn handles_inline_list() {
        let src = "{ hardware.graphics.extraPackages = [ pkgs.rocmPackages.clr.icd ]; }\n";
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["rocmPackages.clr.icd"]);
        assert!(!pkgs[0].migratable, "inline list entries should not be migratable");
    }

    #[test]
    fn ignores_inline_comments() {
        let src = r#"
{
  home.packages = with pkgs; [
    ripgrep  # fast grep
    # fd is great too
    fd
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::HomeManager);
        assert_eq!(names(&pkgs), vec!["ripgrep", "fd"]);
    }

    #[test]
    fn parses_nested_environment_block() {
        let src = r#"
{ config, pkgs, ... }:
{
  environment = {
    sessionVariables.NIXOS_OZONE_WL = "1";

    systemPackages = with pkgs; [
      asusctl
      git
    ];
  };
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["asusctl", "git"]);
    }

    #[test]
    fn skips_flake_input_entries() {
        let src = r#"
{
  environment.systemPackages = with pkgs; [
    asusctl
    inputs.noctalia.packages.${pkgs.stdenv.hostPlatform.system}.default
    inputs.zen-browser.packages."${pkgs.stdenv.hostPlatform.system}".default
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["asusctl"]);
    }

    #[test]
    fn hm_scope_filters_out_system_lists() {
        let src = r#"
{
  environment.systemPackages = with pkgs; [ git ];
  home.packages = with pkgs; [ ripgrep ];
}
"#;
        let pkgs = parse(src, ScanTarget::HomeManager);
        assert_eq!(names(&pkgs), vec!["ripgrep"]);
    }

    #[test]
    fn nixos_scope_filters_out_home_lists() {
        let src = r#"
{
  environment.systemPackages = with pkgs; [ git ];
  home.packages = with pkgs; [ ripgrep ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["git"]);
    }

    #[test]
    fn skips_exclude_packages() {
        let src = r#"
{
  services.xserver = {
    excludePackages = [ pkgs.xterm ];
  };
  environment.systemPackages = with pkgs; [ git ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert_eq!(names(&pkgs), vec!["git"]);
    }

    #[test]
    fn skips_themepackages_complex_override() {
        let src = r#"
{
  boot.plymouth.themePackages = with pkgs; [
    (adi1090x-plymouth-themes.override {
      selected_themes = [
        "cubes"
      ];
    })
  ];
}
"#;
        let pkgs = parse(src, ScanTarget::Nixos);
        assert!(pkgs.is_empty(), "complex override should produce no entries: {:?}", pkgs);
    }

    #[test]
    fn remove_from_source_round_trip() {
        let tmp = std::env::temp_dir().join(format!(
            "nixbox-scan-test-{}.nix",
            std::process::id()
        ));
        let src = r#"{ pkgs, ... }:
{
  home.packages = with pkgs; [
    ripgrep
    fd
    jq
  ];
}
"#;
        fs::write(&tmp, src).unwrap();
        let removed =
            remove_from_source(&tmp, ScanTarget::HomeManager, &["fd".into(), "jq".into()])
                .unwrap();
        assert_eq!(removed, vec!["fd".to_string(), "jq".to_string()]);
        let after = fs::read_to_string(&tmp).unwrap();
        let _ = fs::remove_file(&tmp);
        assert!(after.contains("ripgrep"));
        assert!(!after.contains("fd"));
        assert!(!after.contains("jq"));
    }
}

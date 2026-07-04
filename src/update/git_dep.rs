use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDepSpec {
    pub table_path: &'static [&'static str],
    pub user_agent: String,
}

impl GitDepSpec {
    pub fn new(table_path: &'static [&'static str], user_agent: impl Into<String>) -> Self {
        Self {
            table_path,
            user_agent: user_agent.into(),
        }
    }
}

pub fn read_rev(cargo_toml: &str, spec: &GitDepSpec) -> Result<Option<String>> {
    let doc = cargo_toml
        .parse::<DocumentMut>()
        .context("failed to parse Cargo.toml")?;
    let dep = table_item(doc.as_item(), spec.table_path)?;
    let table = dep.as_inline_table().with_context(|| {
        format!(
            "{} must be an inline table",
            dotted_table_path(spec.table_path)
        )
    })?;
    Ok(table
        .get("rev")
        .and_then(|rev| rev.as_str())
        .map(str::to_owned))
}

pub fn set_rev(cargo_toml: &str, spec: &GitDepSpec, rev: &str) -> Result<String> {
    if !is_sha(rev) {
        anyhow::bail!("rev must be a 40-character hexadecimal SHA: {rev}");
    }

    let mut doc = cargo_toml
        .parse::<DocumentMut>()
        .context("failed to parse Cargo.toml")?;
    let dep = table_item_mut(doc.as_item_mut(), spec.table_path)?;
    if !dep.is_inline_table() {
        anyhow::bail!(
            "{} must be an inline table",
            dotted_table_path(spec.table_path)
        );
    }
    dep["rev"] = value(rev);
    Ok(doc.to_string())
}

pub fn fetch_latest_rev(repo_url: &str, branch: &str, user_agent: &str) -> Result<String> {
    if let Some(api_url) = github_api_commit_url(repo_url, branch) {
        let response: serde_json::Value = ureq::get(&api_url)
            .set("User-Agent", user_agent)
            .call()
            .with_context(|| format!("GitHub API request failed: {api_url}"))?
            .into_json()
            .context("failed to parse GitHub API response")?;
        if let Some(rev) = response["sha"].as_str().filter(|rev| is_sha(rev)) {
            return Ok(rev.to_owned());
        }
    }

    let head = format!("refs/heads/{branch}");
    let stdout = crate::process::capture_stdout(
        "git",
        Path::new("."),
        &["ls-remote", repo_url, head.as_str()],
    )?;
    let rev = stdout
        .split_whitespace()
        .next()
        .context("git ls-remote returned no revision")?;
    if is_sha(rev) {
        Ok(rev.to_owned())
    } else {
        anyhow::bail!("git ls-remote returned invalid revision: {rev}")
    }
}

pub fn github_api_commit_url(repo_url: &str, branch: &str) -> Option<String> {
    let trimmed = repo_url.trim_end_matches('/');
    let path = if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else {
        trimmed.strip_prefix("git@github.com:")?
    };
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    Some(format!(
        "https://api.github.com/repos/{owner}/{repo}/commits/{branch}"
    ))
}

pub fn is_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn table_item<'a>(mut item: &'a Item, path: &[&str]) -> Result<&'a Item> {
    for segment in path {
        item = item
            .get(*segment)
            .with_context(|| format!("{} is missing", dotted_table_path(path)))?;
    }
    Ok(item)
}

fn table_item_mut<'a>(mut item: &'a mut Item, path: &[&str]) -> Result<&'a mut Item> {
    for segment in path {
        item = item
            .get_mut(*segment)
            .with_context(|| format!("{} is missing", dotted_table_path(path)))?;
    }
    Ok(item)
}

fn dotted_table_path(path: &[&str]) -> String {
    path.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: GitDepSpec = GitDepSpec {
        table_path: &["dependencies", "godot"],
        user_agent: String::new(),
    };

    #[test]
    fn converts_https_and_ssh_github_urls() {
        assert_eq!(
            github_api_commit_url("https://github.com/godot-rust/gdext.git", "master"),
            Some("https://api.github.com/repos/godot-rust/gdext/commits/master".to_owned())
        );
        assert_eq!(
            github_api_commit_url("git@github.com:godot-rust/gdext.git", "main"),
            Some("https://api.github.com/repos/godot-rust/gdext/commits/main".to_owned())
        );
    }

    #[test]
    fn inserts_and_reads_inline_table_rev() {
        let input = "[dependencies]\ngodot = { git = \"https://github.com/godot-rust/gdext\" }\n";
        let rev = "2222222222222222222222222222222222222222";
        let output = set_rev(input, &SPEC, rev).unwrap();

        assert!(output.contains("rev = \"2222222222222222222222222222222222222222\""));
        assert_eq!(read_rev(&output, &SPEC).unwrap().as_deref(), Some(rev));
    }

    #[test]
    fn missing_rev_is_none() {
        let input = "[dependencies]\ngodot = { git = \"https://github.com/godot-rust/gdext\" }\n";
        assert_eq!(read_rev(input, &SPEC).unwrap(), None);
    }

    #[test]
    fn validates_sha_shape() {
        assert!(is_sha("abcdefabcdefabcdefabcdefabcdefabcdefabcd"));
        assert!(!is_sha("not-a-sha"));
    }
}

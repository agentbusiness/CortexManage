//! The values a repository's release reads that git does not hold, given by a person (law 7: kept
//! in the Keychain or the Secret Service, never in a file or the graph).
//!
//! A repository reads them its own way, so each place it reads them from is one text a person
//! fills in: each file the profile names (`secret_files`), as the whole file in whatever form the
//! repository reads; with no file named but secrets named, the release's environment, as
//! `NAME=value` lines. The window starts each text with the secret names the profile found
//! (`NAME=` lines). At a release the texts go to the runner, which writes each file into its clean
//! checkout for that release only.

use crate::service::Manager;
use cc_protocol::contracts::manage::{Delivery, Repo};
use cc_protocol::{RpcError, codes};
use serde_json::{Value, json};

const SERVICE: &str = "CortexManage release secrets";

/// The places a release reads its secrets from: each named file, or `""` for the environment.
pub fn places(delivery: &Delivery) -> Vec<String> {
    if !delivery.secret_files.is_empty() {
        return delivery.secret_files.clone();
    }
    if !delivery.secrets.is_empty() {
        return vec![String::new()];
    }
    Vec::new()
}

/// What a place's text starts as: the secret names, one `NAME=` line each.
pub fn template(delivery: &Delivery) -> String {
    delivery.secrets.iter().map(|name| format!("{name}=\n")).collect()
}

fn account(repo: &str, place: &str) -> String {
    format!("{repo}/{}", if place.is_empty() { "environment" } else { place })
}

/// How much of a text one item holds (`security -i` reads lines of about 4000 characters).
const PIECE: usize = 3000;

/// A place's text as kept (in pieces `account#0`, `account#1`, …); empty when none is.
pub fn text(repo: &str, place: &str) -> Result<String, RpcError> {
    let account = account(repo, place);
    let mut hex = String::new();
    for at in 0.. {
        match store::get(SERVICE, &format!("{account}#{at}")).map_err(|problem| RpcError::new(codes::UNAVAILABLE, problem))? {
            Some(piece) => hex.push_str(&piece),
            None => break,
        }
    }
    Ok(unhex(&hex).unwrap_or_default())
}

/// Keep a place's text, or remove it when `text` is empty.
fn keep(repo: &str, place: &str, text: &str) -> Result<(), String> {
    let account = account(repo, place);
    // A file's text ends with one newline, as a file does.
    let hex = if text.trim().is_empty() { String::new() } else { hex(&format!("{}\n", text.trim_end())) };
    let pieces: Vec<&str> = (0..hex.len()).step_by(PIECE).map(|at| &hex[at..(at + PIECE).min(hex.len())]).collect();
    for (at, piece) in pieces.iter().enumerate() {
        store::set(SERVICE, &format!("{account}#{at}"), piece)?;
    }
    // What an earlier, longer text left goes.
    for at in pieces.len().. {
        let name = format!("{account}#{at}");
        if store::get(SERVICE, &name)?.is_none() {
            break;
        }
        store::delete(SERVICE, &name)?;
    }
    Ok(())
}

/// Whether a place holds a text a person gave (not empty, not the names alone).
fn filled(repo: &str, place: &str, template: &str) -> Result<bool, RpcError> {
    let text = text(repo, place)?;
    Ok(!text.trim().is_empty() && text.trim() != template.trim())
}

/// `manage.secrets.get` `{repo}` -> `{places: [{file, template, filled}]}` (never a value).
pub fn get(manager: &Manager, repo_id: &str) -> Result<Value, RpcError> {
    let repo = manager.repo(repo_id)?.repo;
    let template = template(&repo.delivery);
    let mut out = Vec::new();
    for place in places(&repo.delivery) {
        out.push(json!({"file": place, "template": template, "filled": filled(&repo.id, &place, &template)?}));
    }
    Ok(json!({"places": out}))
}

/// `manage.secrets.set` `{repo, file, text}`: keep a place's text; an empty text removes it.
pub fn set(manager: &Manager, params: &Value) -> Result<Value, RpcError> {
    let repo_id = params.get("repo").and_then(Value::as_str).unwrap_or_default();
    let place = params.get("file").and_then(Value::as_str).unwrap_or_default();
    let text = params.get("text").and_then(Value::as_str).unwrap_or_default();
    let repo = manager.repo(repo_id)?.repo;
    if !places(&repo.delivery).iter().any(|known| known == place) {
        return Err(RpcError::invalid_params(manager.t("secrets_no_place", &[])));
    }
    keep(&repo.id, place, text).map_err(|problem| RpcError::new(codes::UNAVAILABLE, problem))?;
    manager.changed("repo", &repo.id, &repo.id);
    get(manager, &repo.id)
}

/// The secret files' texts and the environment's lines for a release. Err, when `required`: the
/// sentence saying which place has not been filled in (otherwise it is left out).
pub fn for_release(manager: &Manager, repo: &Repo, required: bool) -> Result<(Vec<(String, String)>, Vec<(String, String)>), RpcError> {
    let template = template(&repo.delivery);
    let (mut files, mut env) = (Vec::new(), Vec::new());
    for place in places(&repo.delivery) {
        if !filled(&repo.id, &place, &template)? {
            if !required {
                continue;
            }
            let what = if place.is_empty() { manager.t("secrets_environment", &[]) } else { place.clone() };
            return Err(RpcError::new(codes::CONFLICT, manager.t("secrets_missing", &[("place", &what)])));
        }
        let text = text(&repo.id, &place)?;
        if place.is_empty() {
            for line in text.lines() {
                if let Some((name, value)) = line.split_once('=')
                    && !name.trim().is_empty()
                {
                    env.push((name.trim().to_owned(), value.to_owned()));
                }
            }
        } else {
            files.push((place, text));
        }
    }
    Ok((files, env))
}

/// The Keychain takes plain text only; a file's text (lines, quotes) is kept as hex.
fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(hex: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|at| hex.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok())).collect();
    String::from_utf8(bytes?).ok()
}

/// Generic passwords in the macOS login Keychain, through Apple's `/usr/bin/security` (as the
/// vcs-git plugin keeps its tokens: the same program makes and reads every item, so an update of
/// the app asks nothing). The secret goes on stdin, never on a command line.
#[cfg(target_os = "macos")]
mod store {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const SECURITY: &str = "/usr/bin/security";
    const NOT_FOUND: i32 = 44;

    pub fn get(service: &str, account: &str) -> Result<Option<String>, String> {
        let output = Command::new(SECURITY).args(["find-generic-password", "-s", service, "-a", account, "-w"]).stdin(Stdio::null()).output().map_err(|error| format!("cannot run {SECURITY}: {error}"))?;
        if output.status.success() {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_owned()));
        }
        if output.status.code() == Some(NOT_FOUND) {
            return Ok(None);
        }
        Err(format!("Keychain: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }

    pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
        if account.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
            return Err("the file's name has a quote, backslash or control character".to_owned());
        }
        let mut child = Command::new(SECURITY).arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|error| format!("cannot run {SECURITY}: {error}"))?;
        let line = format!("add-generic-password -U -s \"{service}\" -a \"{account}\" -w \"{secret}\"\n");
        child.stdin.take().ok_or("no stdin")?.write_all(line.as_bytes()).map_err(|error| format!("Keychain: {error}"))?;
        let output = child.wait_with_output().map_err(|error| format!("Keychain: {error}"))?;
        let errors = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() || !errors.trim().is_empty() {
            return Err(format!("Keychain: {}", errors.trim()));
        }
        Ok(())
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let output = Command::new(SECURITY).args(["delete-generic-password", "-s", service, "-a", account]).stdin(Stdio::null()).output().map_err(|error| format!("cannot run {SECURITY}: {error}"))?;
        if output.status.success() || output.status.code() == Some(NOT_FOUND) {
            return Ok(());
        }
        Err(format!("Keychain: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

/// The Secret Service, through `secret-tool` (libsecret).
#[cfg(not(target_os = "macos"))]
mod store {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const TOOL: &str = "secret-tool";

    fn run(args: &[&str], input: Option<&str>) -> Result<std::process::Output, String> {
        let mut child = Command::new(TOOL)
            .args(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run {TOOL} ({error}); install libsecret-tools"))?;
        if let Some(input) = input {
            child.stdin.take().ok_or("no stdin")?.write_all(input.as_bytes()).map_err(|error| format!("{TOOL}: {error}"))?;
        }
        child.wait_with_output().map_err(|error| format!("{TOOL}: {error}"))
    }

    pub fn get(service: &str, account: &str) -> Result<Option<String>, String> {
        let output = run(&["lookup", "service", service, "account", account], None)?;
        let text = String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_owned();
        Ok(if output.status.success() && !text.is_empty() { Some(text) } else { None })
    }

    pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
        let output = run(&["store", "--label", service, "service", service, "account", account], Some(secret))?;
        if output.status.success() { Ok(()) } else { Err(format!("{TOOL}: {}", String::from_utf8_lossy(&output.stderr).trim())) }
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let _ = run(&["clear", "service", service, "account", account], None)?;
        Ok(())
    }
}

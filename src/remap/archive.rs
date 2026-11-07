use super::{RemapReport, class};
use crate::{
    Result, input::safe_entry_name, mappings::MappingPlan, naming::NamePlan, program::Program,
};
use anyhow::{Context, ensure};
use std::{collections::BTreeMap, fs, io::Write, path::Path, sync::Arc};

pub(super) fn write(
    destination: &Path,
    program: &Program<'_>,
    names: &NamePlan,
    mappings: &MappingPlan,
) -> Result<RemapReport> {
    let input = program
        .input
        .and_then(|input| input.native())
        .context("native remapping requires Input::read_for_remap and Program::from_input")?;
    let mut report = RemapReport::default();
    let mut output = BTreeMap::new();
    let mut resources = BTreeMap::new();
    for owner in 0..program.own_count() {
        let original = &program.class(owner).name;
        let bytes = class::remap(
            &input.classes[original],
            program,
            owner,
            names,
            mappings,
            &mut report.warnings,
        )
        .with_context(|| format!("remap {original}"))?;
        let mapped = format!("{}.class", names.class_name(owner));
        insert(&mut output, mapped, bytes.into())?;
        report.classes += 1;
    }
    for (name, bytes) in &input.resources {
        if signature_file(name) {
            report
                .warnings
                .push(format!("removed invalidated JAR signature {name}"));
        } else {
            let bytes = if name.eq_ignore_ascii_case("META-INF/MANIFEST.MF") {
                manifest(bytes, names).into()
            } else {
                bytes.clone()
            };
            resources.insert(name.clone(), bytes);
        }
    }
    // Keep original paths for absolute lookups, and alias moved packages for
    // Class.getResourceAsStream's relative paths, including computed strings.
    // Conflicting aliases fail rather than choosing a resource nondeterministically.
    for (name, bytes) in &resources {
        insert(&mut output, name.clone(), bytes.clone())?;
    }
    for (original, emitted) in names.package_moves() {
        let prefix = if original.is_empty() {
            String::new()
        } else {
            format!("{original}/")
        };
        for (name, bytes) in &resources {
            if name.to_ascii_uppercase().starts_with("META-INF/") {
                continue;
            }
            let Some(tail) = name.strip_prefix(&prefix) else {
                continue;
            };
            let alias = if emitted.is_empty() {
                tail.to_owned()
            } else {
                format!("{emitted}/{tail}")
            };
            insert(&mut output, alias, bytes.clone())?;
        }
    }
    report.resources = output.len() - report.classes;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut jar = zip::ZipWriter::new(temporary.as_file_mut());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in output {
            jar.start_file(name, options)?;
            jar.write_all(&bytes)?;
        }
        jar.finish()?;
    }
    temporary
        .persist(destination)
        .map_err(|error| error.error)
        .with_context(|| format!("write {}", destination.display()))?;
    report.warnings.sort();
    report.warnings.dedup();
    Ok(report)
}

fn insert(entries: &mut BTreeMap<String, Arc<[u8]>>, name: String, bytes: Arc<[u8]>) -> Result<()> {
    safe_entry_name(&name)?;
    if let Some(previous) = entries.get(&name) {
        ensure!(previous == &bytes, "remapped JAR entry collision at {name}");
    } else {
        entries.insert(name, bytes);
    }
    Ok(())
}

fn signature_file(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let Some(leaf) = upper.strip_prefix("META-INF/") else {
        return false;
    };
    !leaf.contains('/')
        && (leaf.starts_with("SIG-")
            || [".SF", ".RSA", ".DSA", ".EC"]
                .iter()
                .any(|suffix| leaf.ends_with(suffix)))
}

fn manifest(bytes: &[u8], names: &NamePlan) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    for line in bytes.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(rest) = line.strip_prefix(b" ") {
            if let Some(previous) = lines.last_mut() {
                previous.extend_from_slice(rest);
            }
        } else {
            lines.push(line.to_vec());
        }
    }
    let mut result = Vec::new();
    for line in lines {
        let Some(colon) = line.iter().position(|&byte| byte == b':') else {
            result.extend_from_slice(&line);
            result.extend_from_slice(b"\r\n");
            continue;
        };
        let key = String::from_utf8_lossy(&line[..colon]).to_ascii_lowercase();
        if key.ends_with("-digest")
            || key.contains("-digest-")
            || key == "signature-version"
            || key == "magic"
        {
            continue;
        }
        let raw = line[colon + 1..].trim_ascii_start();
        let mut changed = None;
        if let Ok(value) = std::str::from_utf8(raw) {
            if key == "main-class" || key == "midlet-class" {
                let original = value.trim().replace('.', "/");
                let mapped = names.class_name_for(&original);
                if mapped != original {
                    changed = Some(mapped.replace('/', "."));
                }
            } else if key.strip_prefix("midlet-").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
            }) {
                if let Some((prefix, original)) = value.rsplit_once(',') {
                    let original = original.trim().replace('.', "/");
                    let mapped = names.class_name_for(&original);
                    if mapped != original {
                        changed = Some(format!("{prefix}, {}", mapped.replace('/', ".")));
                    }
                }
            } else if key == "name" && value.ends_with(".class") {
                let original = &value[..value.len() - 6];
                let mapped = names.class_name_for(original);
                if mapped != original {
                    changed = Some(format!("{mapped}.class"));
                }
            }
        }
        let line = if let Some(value) = changed {
            let mut line = line[..=colon].to_vec();
            line.push(b' ');
            line.extend_from_slice(value.as_bytes());
            line
        } else {
            line
        };
        for (index, part) in line.chunks(70).enumerate() {
            if index != 0 {
                result.push(b' ');
            }
            result.extend_from_slice(part);
            result.extend_from_slice(b"\r\n");
        }
    }
    if !result.ends_with(b"\r\n\r\n") {
        result.extend_from_slice(b"\r\n");
    }
    result
}

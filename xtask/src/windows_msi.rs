use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const COMPONENT_REGISTRY_KEY: &str = r"Software\io.github.airwiki\AirWiki\Components";

#[derive(Debug)]
struct ResourceFile {
    source: PathBuf,
    destination: String,
    digest: String,
}

#[derive(Debug, Default)]
struct ResourceDirectory {
    children: BTreeMap<String, ResourceDirectory>,
    files: Vec<ResourceFile>,
}

pub(crate) fn generate_workspace_fragment() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must have a workspace parent")?;
    let target = root.join("target");
    let release = target.join("x86_64-pc-windows-msvc").join("release");
    let runtime = root.join("resources").join("llama").join("windows-x64");
    let mcpb = target
        .join("mcpb")
        .join("x86_64-pc-windows-msvc")
        .join("airwiki-claude.mcpb");
    let output = target.join("windows-msi-resources.wxs");
    generate_fragment(root, &target, &release, &runtime, &mcpb, &output)
}

fn generate_fragment(
    root: &Path,
    target: &Path,
    release: &Path,
    runtime: &Path,
    mcpb: &Path,
    output: &Path,
) -> Result<()> {
    let mut resources = BTreeMap::new();
    add_file(
        &mut resources,
        &release.join("airwiki-mcp-bridge.exe"),
        "integrations/bridge/airwiki-mcp-bridge.exe",
    )?;
    add_file(
        &mut resources,
        &release.join("airwiki-windows-firewall-helper.exe"),
        "airwiki-windows-firewall-helper.exe",
    )?;
    add_file(&mut resources, mcpb, "integrations/airwiki-claude.mcpb")?;
    add_file(
        &mut resources,
        &root.join("THIRD_PARTY_NOTICES.md"),
        "THIRD_PARTY_NOTICES.md",
    )?;
    add_file(&mut resources, &root.join("LICENSE"), "LICENSE")?;
    add_tree(&mut resources, runtime, "llama")?;
    add_tree(&mut resources, &root.join("resources/licenses"), "licenses")?;

    ensure!(!resources.is_empty(), "Windows MSI resource set is empty");
    let document = render_fragment(resources.into_values())?;
    write_fragment(target, output, document.as_bytes())
}

fn add_tree(
    resources: &mut BTreeMap<String, ResourceFile>,
    source_root: &Path,
    destination_root: &str,
) -> Result<()> {
    ensure_regular_directory(source_root, "Windows MSI resource directory")?;
    let mut pending = vec![(source_root.to_path_buf(), destination_root.to_owned())];
    while let Some((source, destination)) = pending.pop() {
        let mut entries = fs::read_dir(&source)
            .with_context(|| format!("reading MSI resource directory {}", source.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("MSI resource name is not Unicode"))?;
            validate_destination_segment(&name)?;
            let destination = format!("{destination}/{name}");
            let metadata = fs::symlink_metadata(entry.path())
                .with_context(|| format!("inspecting MSI resource {}", entry.path().display()))?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "MSI resources must not contain symbolic links"
            );
            if metadata.is_dir() {
                pending.push((entry.path(), destination));
            } else if metadata.is_file() {
                add_file(resources, &entry.path(), &destination)?;
            } else {
                bail!("MSI resources must contain only regular files and directories");
            }
        }
    }
    Ok(())
}

fn add_file(
    resources: &mut BTreeMap<String, ResourceFile>,
    source: &Path,
    destination: &str,
) -> Result<()> {
    ensure_regular_file(source, "Windows MSI resource")?;
    ensure!(
        source.is_absolute(),
        "Windows MSI resource source must be absolute: {}",
        source.display()
    );
    validate_destination(destination)?;
    let digest = hex::encode(Sha256::digest(destination.as_bytes()));
    let resource = ResourceFile {
        // Windows canonicalization adds a verbatim `\\?\` prefix that WiX 3
        // Candle does not accept as a File/@Source path. The caller supplies
        // workspace-rooted absolute paths and every traversed entry has already
        // been checked with symlink_metadata.
        source: source.to_path_buf(),
        destination: destination.to_owned(),
        digest,
    };
    ensure!(
        resources.insert(destination.to_owned(), resource).is_none(),
        "duplicate MSI resource destination: {destination}"
    );
    Ok(())
}

fn ensure_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {label} {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} is not a regular file: {}",
        path.display()
    );
    Ok(())
}

fn ensure_regular_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {label} {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} is not a regular directory: {}",
        path.display()
    );
    Ok(())
}

fn validate_destination(destination: &str) -> Result<()> {
    ensure!(
        !destination.is_empty() && !destination.contains('\\'),
        "MSI resource destination must use non-empty forward-slash paths"
    );
    let path = Path::new(destination);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "MSI resource destination is not a safe relative path: {destination}"
    );
    for segment in destination.split('/') {
        validate_destination_segment(segment)?;
    }
    Ok(())
}

fn validate_destination_segment(segment: &str) -> Result<()> {
    ensure!(
        !segment.is_empty() && segment != "." && segment != "..",
        "MSI resource destination contains an invalid segment"
    );
    Ok(())
}

fn render_fragment(resources: impl IntoIterator<Item = ResourceFile>) -> Result<String> {
    let mut root = ResourceDirectory::default();
    for resource in resources {
        let segments = resource.destination.split('/').collect::<Vec<_>>();
        let (file_name, directories) = segments
            .split_last()
            .context("MSI resource destination has no file name")?;
        let mut directory = &mut root;
        for segment in directories {
            directory = directory.children.entry((*segment).to_owned()).or_default();
        }
        directory.files.push(ResourceFile {
            source: resource.source,
            destination: (*file_name).to_owned(),
            digest: resource.digest,
        });
    }

    let mut component_ids = BTreeSet::new();
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Wix xmlns=\"http://schemas.microsoft.com/wix/2006/wi\">\n  <Fragment>\n    <DirectoryRef Id=\"INSTALLDIR\">\n",
    );
    render_directory_contents(&root, "", 6, &mut component_ids, &mut xml)?;
    xml.push_str(
        "    </DirectoryRef>\n  </Fragment>\n  <Fragment>\n    <ComponentGroup Id=\"AirWikiResources\">\n",
    );
    for id in component_ids {
        xml.push_str(&format!("      <ComponentRef Id=\"{id}\" />\n"));
    }
    xml.push_str("    </ComponentGroup>\n  </Fragment>\n</Wix>\n");
    Ok(xml)
}

fn render_directory_contents(
    directory: &ResourceDirectory,
    destination: &str,
    indent: usize,
    component_ids: &mut BTreeSet<String>,
    xml: &mut String,
) -> Result<()> {
    for file in &directory.files {
        let id = file_component_id(&file.digest);
        component_ids.insert(id.clone());
        let guid = stable_guid("file", &file.digest);
        let source = escape_xml_attribute(&file.source.to_string_lossy());
        let name = escape_xml_attribute(&file.destination);
        let spaces = " ".repeat(indent);
        xml.push_str(&format!(
            "{spaces}<Component Id=\"{id}\" Guid=\"{guid}\" Win64=\"$(var.Win64)\">\n{spaces}  <File Id=\"{}\" Name=\"{name}\" Source=\"{source}\" />\n{spaces}  <RegistryValue Root=\"HKCU\" Key=\"{COMPONENT_REGISTRY_KEY}\" Name=\"File_{}\" Type=\"integer\" Value=\"1\" KeyPath=\"yes\" />\n{spaces}</Component>\n",
            file_id(&file.digest),
            file.digest,
        ));
    }
    for (name, child) in &directory.children {
        let child_destination = if destination.is_empty() {
            name.clone()
        } else {
            format!("{destination}/{name}")
        };
        let digest = hex::encode(Sha256::digest(child_destination.as_bytes()));
        let directory_id = directory_id(&digest);
        let component_id = directory_component_id(&digest);
        component_ids.insert(component_id.clone());
        let guid = stable_guid("directory", &digest);
        let spaces = " ".repeat(indent);
        xml.push_str(&format!(
            "{spaces}<Directory Id=\"{directory_id}\" Name=\"{}\">\n{spaces}  <Component Id=\"{component_id}\" Guid=\"{guid}\" Win64=\"$(var.Win64)\">\n{spaces}    <RemoveFolder Id=\"{}\" On=\"uninstall\" />\n{spaces}    <RegistryValue Root=\"HKCU\" Key=\"{COMPONENT_REGISTRY_KEY}\" Name=\"Directory_{digest}\" Type=\"integer\" Value=\"1\" KeyPath=\"yes\" />\n{spaces}  </Component>\n",
            escape_xml_attribute(name),
            remove_folder_id(&digest),
        ));
        render_directory_contents(child, &child_destination, indent + 2, component_ids, xml)?;
        xml.push_str(&format!("{spaces}</Directory>\n"));
    }
    Ok(())
}

fn stable_guid(kind: &str, digest: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("https://airwiki.dev/windows-msi/{kind}/{digest}").as_bytes(),
    )
}

fn file_component_id(digest: &str) -> String {
    format!("AirWikiFileComponent_{}", &digest[..32])
}

fn directory_component_id(digest: &str) -> String {
    format!("AirWikiDirComponent_{}", &digest[..32])
}

fn file_id(digest: &str) -> String {
    format!("AirWikiFile_{}", &digest[..32])
}

fn directory_id(digest: &str) -> String {
    format!("AirWikiDir_{}", &digest[..32])
}

fn remove_folder_id(digest: &str) -> String {
    format!("AirWikiRemoveDir_{}", &digest[..32])
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn write_fragment(target: &Path, output: &Path, contents: &[u8]) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("creating Windows MSI target root {}", target.display()))?;
    let target = target
        .canonicalize()
        .context("canonicalizing Windows MSI target root")?;
    let parent = output
        .parent()
        .context("MSI fragment output has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating MSI fragment parent {}", parent.display()))?;
    let parent = parent
        .canonicalize()
        .context("canonicalizing MSI fragment parent")?;
    ensure!(
        parent.starts_with(&target),
        "MSI fragment output must remain below target"
    );
    let staging = parent.join(format!(".airwiki-msi-{}.wxs", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .context("creating MSI fragment staging")?;
        temporary
            .write_all(contents)
            .context("writing MSI fragment staging")?;
        temporary
            .sync_all()
            .context("syncing MSI fragment staging")?;
        if output.exists() {
            ensure_regular_file(output, "existing MSI fragment output")?;
            fs::remove_file(output).context("removing previous MSI fragment output")?;
        }
        fs::rename(&staging, output).context("publishing MSI fragment")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_uses_hkcu_keypaths_and_removes_profile_directories() {
        let destination = "licenses/A&B.txt";
        let digest = hex::encode(Sha256::digest(destination.as_bytes()));
        let xml = render_fragment([ResourceFile {
            source: PathBuf::from(r"C:\build\A&B.txt"),
            destination: destination.to_owned(),
            digest: digest.clone(),
        }])
        .expect("fixture should render");

        assert!(xml.contains("<ComponentGroup Id=\"AirWikiResources\">"));
        assert_eq!(xml.matches("Root=\"HKCU\"").count(), 2);
        assert_eq!(xml.matches("KeyPath=\"yes\"").count(), 2);
        assert_eq!(xml.matches("<RemoveFolder ").count(), 1);
        assert!(xml.contains(r#"Source="C:\build\A&amp;B.txt""#));
        assert!(xml.contains(&file_component_id(&digest)));
        assert!(!xml.contains("File KeyPath="));
    }

    #[test]
    fn destination_validation_rejects_traversal_and_backslashes() {
        assert!(validate_destination("licenses/NOTICE.txt").is_ok());
        assert!(validate_destination("../NOTICE.txt").is_err());
        assert!(validate_destination(r"licenses\NOTICE.txt").is_err());
        assert!(validate_destination("licenses//NOTICE.txt").is_err());
    }

    #[test]
    fn workspace_fragment_is_reproducible_and_contains_every_required_payload() {
        let temporary = tempfile::tempdir().expect("temporary workspace should exist");
        let root = temporary.path();
        let target = root.join("target");
        let release = target.join("release");
        let runtime = root.join("runtime");
        let licenses = root.join("resources/licenses");
        let mcpb = target.join("airwiki-claude.mcpb");
        let output = target.join("windows-msi-resources.wxs");
        for directory in [&release, &runtime, &licenses] {
            fs::create_dir_all(directory).expect("fixture directory should be created");
        }
        for (path, contents) in [
            (release.join("airwiki-mcp-bridge.exe"), b"bridge".as_slice()),
            (
                release.join("airwiki-windows-firewall-helper.exe"),
                b"helper".as_slice(),
            ),
            (runtime.join("llama-server.exe"), b"runtime".as_slice()),
            (runtime.join("BUILD-MANIFEST.json"), b"manifest".as_slice()),
            (licenses.join("Apache-2.0.txt"), b"license".as_slice()),
            (mcpb.clone(), b"mcpb".as_slice()),
            (root.join("THIRD_PARTY_NOTICES.md"), b"notices".as_slice()),
            (root.join("LICENSE"), b"project license".as_slice()),
        ] {
            fs::write(path, contents).expect("fixture file should be written");
        }

        generate_fragment(root, &target, &release, &runtime, &mcpb, &output)
            .expect("first fragment should be generated");
        let first = fs::read_to_string(&output).expect("first fragment should be readable");
        generate_fragment(root, &target, &release, &runtime, &mcpb, &output)
            .expect("second fragment should be generated");
        let second = fs::read_to_string(&output).expect("second fragment should be readable");

        assert_eq!(first, second);
        for destination in [
            "airwiki-mcp-bridge.exe",
            "airwiki-windows-firewall-helper.exe",
            "airwiki-claude.mcpb",
            "llama-server.exe",
            "BUILD-MANIFEST.json",
            "Apache-2.0.txt",
            "THIRD_PARTY_NOTICES.md",
            "LICENSE",
        ] {
            assert!(first.contains(destination), "missing {destination}");
        }
        assert_eq!(first.matches("KeyPath=\"yes\"").count(), 12);
        assert_eq!(first.matches("<RemoveFolder ").count(), 4);
    }
}

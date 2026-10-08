#[cfg(windows)]
fn main() {
    use std::{env, fs, path::PathBuf, process::Command};
    println!("cargo:rerun-if-changed=assets\\app-icon.ico");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Repository directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Build output directory"));
    let resource = output.join("app-icon.rc");
    let compiled = output.join("app-icon.res");
    let icon = root.join("assets").join("app-icon.ico");
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|path| path.join("rc.exe")));
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let kits = PathBuf::from(program_files)
            .join("Windows Kits")
            .join("10")
            .join("bin");
        if kits.is_dir() {
            let mut versions: Vec<_> = fs::read_dir(&kits)
                .expect("Read installed Windows SDKs")
                .map(|entry| entry.expect("Inspect Windows SDK").path())
                .collect();
            versions.sort();
            candidates.extend(
                versions
                    .into_iter()
                    .rev()
                    .map(|version| version.join("x64").join("rc.exe")),
            );
        }
    }
    let compiler = candidates
        .into_iter()
        .find(|path| path.is_file())
        .expect("Windows SDK resource compiler rc.exe is required to embed the app icon");
    let icon = icon
        .to_str()
        .expect("Icon path must be Unicode")
        .replace('\\', "\\\\");
    fs::write(
        &resource,
        format!("#pragma code_page(65001)\n1 ICON \"{icon}\"\n"),
    )
    .expect("Write icon resource");
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&compiled)
        .arg(&resource)
        .status()
        .expect("Run Windows resource compiler");
    assert!(status.success(), "Windows icon resource compilation failed");
    println!(
        "cargo:rustc-link-arg-bin=tomas-commander={}",
        compiled.display()
    );
}

#[cfg(not(windows))]
fn main() {}

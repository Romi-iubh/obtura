//! Windows only: puts Obtura's icon, its version details and its manifest into obtura.exe (a
//! resource file, compiled with the cross toolchain's windres). Nothing to do on Linux.
//!
//! The icon comes from `OBTURA_WINDOWS_ICON` (an .ico file, made by
//! packaging/windows/build-in-container.sh); without it obtura.exe has Windows' plain icon.

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=OBTURA_WINDOWS_ICON");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let version = env::var("CARGO_PKG_VERSION").expect("version");
    let mut parts: Vec<u32> = version.split(['.', '-']).filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let numbers = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let manifest = out.join("obtura.exe.manifest");
    std::fs::write(&manifest, MANIFEST).expect("manifest");
    let mut rc = String::new();
    if let Some(icon) = env::var_os("OBTURA_WINDOWS_ICON") {
        rc.push_str(&format!("1 ICON \"{}\"\n", PathBuf::from(icon).display().to_string().replace('\\', "/")));
    }
    rc.push_str(&format!("1 24 \"{}\"\n", manifest.display().to_string().replace('\\', "/")));
    rc.push_str(&format!(
        r#"1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Rosaire"
      VALUE "FileDescription", "Obtura"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "obtura"
      VALUE "LegalCopyright", "2026 Rosaire. GNU AGPL 3.0 or later."
      VALUE "OriginalFilename", "obtura.exe"
      VALUE "ProductName", "Obtura"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    ));
    let rc_file = out.join("obtura.rc");
    std::fs::write(&rc_file, rc).expect("rc file");
    let object = out.join("obtura-resources.o");
    let windres = env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    let status = Command::new(&windres).arg("-i").arg(&rc_file).arg("-o").arg(&object).status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", object.display()),
        _ => println!("cargo:warning={windres} failed: obtura.exe gets no icon or version details"),
    }
}

/// Windows 10 and 11, sharp on every screen scale, UTF-8.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="com.grasshillsco.Obtura" version="1.0.0.0"/>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
</assembly>
"#;

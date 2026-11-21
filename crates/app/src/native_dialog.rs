use std::path::PathBuf;
use std::process::Command;

/// ゼロ依存・`unsafe` ゼロの OS ネイティブ ファイル選択ダイアログ。
/// - Windows 11: PowerShell + `System.Windows.Forms.OpenFileDialog`（環境変数経由で安全に引数渡し）
/// - Linux: `zenity` → `kdialog` の順にフォールバック
/// - ヘッドレス CI 等でダイアログ起動不可の場合は `None` を返す
pub fn pick_file(title: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        pick_file_windows(title, extensions)
    }
    #[cfg(not(windows))]
    {
        pick_file_unix(title, extensions)
    }
}

/// ゼロ依存・`unsafe` ゼロの OS ネイティブ フォルダ選択ダイアログ。
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        pick_folder_windows(title)
    }
    #[cfg(not(windows))]
    {
        pick_folder_unix(title)
    }
}

#[cfg(windows)]
fn pick_file_windows(title: &str, extensions: &[&str]) -> Option<PathBuf> {
    let filter = build_windows_filter(extensions);
    let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.OpenFileDialog
$d.Title = $env:VE_DLG_TITLE
$d.Filter = $env:VE_DLG_FILTER
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $d.FileName
}
"#;
    run_powershell_dialog(script, title, Some(&filter))
}

#[cfg(windows)]
fn pick_folder_windows(title: &str) -> Option<PathBuf> {
    let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.FolderBrowserDialog
$d.Description = $env:VE_DLG_TITLE
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $d.SelectedPath
}
"#;
    run_powershell_dialog(script, title, None)
}

#[cfg(windows)]
fn run_powershell_dialog(script: &str, title: &str, filter: Option<&str>) -> Option<PathBuf> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-STA", "-Command", script])
        .env("VE_DLG_TITLE", title);
    if let Some(f) = filter {
        cmd.env("VE_DLG_FILTER", f);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

#[cfg(not(windows))]
fn pick_file_unix(title: &str, extensions: &[&str]) -> Option<PathBuf> {
    let mut zenity = Command::new("zenity");
    zenity.args(["--file-selection", "--title", title]);
    if !extensions.is_empty() {
        let patterns: Vec<String> = extensions
            .iter()
            .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(|e| format!("*.{e}"))
            .collect();
        if !patterns.is_empty() {
            zenity.arg(format!("--file-filter=Media | {}", patterns.join(" ")));
            zenity.arg("--file-filter=All files | *");
        }
    }
    if let Ok(out) = zenity.output()
        && out.status.success()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }

    let mut kdialog = Command::new("kdialog");
    kdialog.args(["--getopenfilename", ".", "--title", title]);
    if let Ok(out) = kdialog.output()
        && out.status.success()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

#[cfg(not(windows))]
fn pick_folder_unix(title: &str) -> Option<PathBuf> {
    if let Ok(out) = Command::new("zenity")
        .args(["--file-selection", "--directory", "--title", title])
        .output()
        && out.status.success()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(out) = Command::new("kdialog")
        .args(["--getexistingdirectory", ".", "--title", title])
        .output()
        && out.status.success()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

/// Windows OpenFileDialog 用フィルタ文字列を安全に構築する（英数字拡張子のみ許可）
#[cfg_attr(not(windows), allow(dead_code))]
fn build_windows_filter(extensions: &[&str]) -> String {
    let patterns: Vec<String> = extensions
        .iter()
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!("*.{e}"))
        .collect();
    if patterns.is_empty() {
        "All Files (*.*)|*.*".to_string()
    } else {
        let joined = patterns.join(";");
        format!("Supported Files ({joined})|{joined}|All Files (*.*)|*.*")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_filter_sanitizes_extensions() {
        assert_eq!(
            build_windows_filter(&["wav", "mp4", "../bad;rm"]),
            "Supported Files (*.wav;*.mp4)|*.wav;*.mp4|All Files (*.*)|*.*"
        );
        assert_eq!(build_windows_filter(&[]), "All Files (*.*)|*.*");
    }
}

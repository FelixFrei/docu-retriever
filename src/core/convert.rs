use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use walkdir::WalkDir;

use crate::models::AppError;

#[derive(Debug, Clone)]
pub struct ConvertSummary {
    pub converted_files: usize,
    pub output_dir: PathBuf,
}

pub fn convert_asciidoc_to_markdown(
    input: &Path,
    output: &Path,
) -> Result<ConvertSummary, AppError> {
    if !input.exists() {
        return Err(AppError::invalid_input(format!(
            "Input path does not exist: {}",
            input.display()
        )));
    }
    if !input.is_dir() {
        return Err(AppError::invalid_input(format!(
            "Input path must be a directory: {}",
            input.display()
        )));
    }

    ensure_tool_available("asciidoctor")?;
    ensure_tool_available("pandoc")?;

    fs::create_dir_all(output).map_err(AppError::internal)?;

    let mut converted = 0usize;

    for entry in WalkDir::new(input)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| is_asciidoc(e.path()))
    {
        let rel = entry
            .path()
            .strip_prefix(input)
            .map_err(AppError::internal)?;
        let out_file = output.join(rel).with_extension("md");
        if let Some(parent) = out_file.parent() {
            fs::create_dir_all(parent).map_err(AppError::internal)?;
        }

        convert_file(entry.path(), input, &out_file)?;
        converted += 1;
    }

    if converted == 0 {
        return Err(AppError::invalid_input(
            "No AsciiDoc files found (*.adoc, *.asciidoc)",
        ));
    }

    Ok(ConvertSummary {
        converted_files: converted,
        output_dir: output.to_path_buf(),
    })
}

fn ensure_tool_available(tool: &str) -> Result<(), AppError> {
    let status = Command::new(tool)
        .arg("--version")
        .status()
        .map_err(|err| {
            AppError::invalid_input(format!(
                "Required tool '{tool}' is not available in PATH: {err}"
            ))
        })?;

    if !status.success() {
        return Err(AppError::invalid_input(format!(
            "Required tool '{tool}' is installed but not executable"
        )));
    }

    Ok(())
}

fn convert_file(input_file: &Path, base_dir: &Path, output_file: &Path) -> Result<(), AppError> {
    let temp_docbook =
        std::env::temp_dir().join(format!("docu-retriever-{}.xml", uuid::Uuid::new_v4()));

    let asciidoctor_out = Command::new("asciidoctor")
        .arg("-b")
        .arg("docbook5")
        .arg("-o")
        .arg(&temp_docbook)
        .arg("--base-dir")
        .arg(base_dir)
        .arg(input_file)
        .output()
        .map_err(AppError::internal)?;

    if !asciidoctor_out.status.success() {
        let stderr = String::from_utf8_lossy(&asciidoctor_out.stderr);
        let _ = fs::remove_file(&temp_docbook);
        return Err(AppError::invalid_input(format!(
            "asciidoctor failed for '{}': {}",
            input_file.display(),
            stderr.trim()
        )));
    }

    let pandoc_out = Command::new("pandoc")
        .arg("-f")
        .arg("docbook")
        .arg("-t")
        .arg("gfm")
        .arg("--wrap=none")
        .arg(&temp_docbook)
        .arg("-o")
        .arg(output_file)
        .output()
        .map_err(AppError::internal)?;

    let _ = fs::remove_file(&temp_docbook);

    if !pandoc_out.status.success() {
        let stderr = String::from_utf8_lossy(&pandoc_out.stderr);
        return Err(AppError::invalid_input(format!(
            "pandoc failed for '{}': {}",
            input_file.display(),
            stderr.trim()
        )));
    }

    Ok(())
}

fn is_asciidoc(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("adoc") | Some("asciidoc")
    )
}

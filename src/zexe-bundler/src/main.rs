use anyhow::{Context, Result};
use clap::Parser;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{PathBuf, Path};
use flate2::Compression;
use flate2::write::ZlibEncoder;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const FOOTER_MAGIC: &[u8; 4] = b"ZXND";

fn compress_data(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Input Spectrum snapshot (.sna, .z80 or .szx)
    input: PathBuf,

    /// Output EXE file (Optional, defaults to input name)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Path to the runner executable template (Optional)
    #[arg(short, long)]
    runner: Option<PathBuf>,

    /// Path to a GLSL shader to embed (Optional) (default search: input_name.glsl, shader.glsl)
    #[arg(short, long)]
    shader: Option<PathBuf>,

    /// Path to a POK file to embed (Optional) (default: input_name.pok)
    #[arg(short, long)]
    pokes: Option<PathBuf>,

    /// Path to a JSON config file to embed (Optional) (default search: input_name.json, config.json)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let output_path = if let Some(out) = &args.output {
        out.clone()
    } else {
        let mut out = args.input.clone();
        if cfg!(windows) {
            out.set_extension("exe");
        } else {
            out.set_extension("");
        }
        out
    };

    println!("Bundling {:?}...", args.input);

    // 1. Read Snapshot
    let mut input_file = File::open(&args.input).context("Failed to open input snapshot")?;
    let mut snapshot_data = Vec::new();
    input_file.read_to_end(&mut snapshot_data)?;
    anyhow::ensure!(!snapshot_data.is_empty(), "The snapshot is empty");
    println!("Snapshot size: {} bytes", snapshot_data.len());

    // 2. Read Runner
    let runner = match args.runner {
        Some(path) => path,
        None => {
            let name = format!("zexe-runner{}", std::env::consts::EXE_SUFFIX);
            let sibling = std::env::current_exe()?.with_file_name(&name);
            if sibling.is_file() { sibling } else { PathBuf::from(name) }
        }
    };
    let mut runner_file = File::open(&runner).context("Failed to open runner executable")?;
    let mut runner_data = Vec::new();
    runner_file.read_to_end(&mut runner_data)?;
    println!("Runner template size: {} bytes", runner_data.len());

    // 3. Optional Shader
    let mut shader_data = Vec::new();
    let shader_path = if let Some(path) = args.shader {
        Some(path)
    } else {
        let mut auto_path = args.input.clone();
        auto_path.set_extension("glsl");
        if auto_path.exists() {
            Some(auto_path)
        } else {
            // Fallback: try shader.glsl in the current working directory
            let global_shader = Path::new("shader.glsl");
            if global_shader.exists() { Some(global_shader.to_path_buf()) } else { None }
        }
    };

    if let Some(path) = shader_path {
        println!("Embedding shader from {:?}...", path);
        let mut shader_file = File::open(path).context("Failed to open shader file")?;
        shader_file.read_to_end(&mut shader_data)?;
    }

    // 4. Optional Pokes
    let mut pokes_data = Vec::new();
    let pokes_path = if let Some(path) = args.pokes {
        Some(path)
    } else {
        let mut auto_path = args.input.clone();
        auto_path.set_extension("pok");
        if auto_path.exists() { Some(auto_path) } else { None }
    };

    if let Some(path) = pokes_path {
        println!("Embedding pokes from {:?}...", path);
        let mut pokes_file = File::open(path).context("Failed to open pokes file")?;
        pokes_file.read_to_end(&mut pokes_data)?;
    }

    // 5. Optional Config
    let mut config_data = Vec::new();
    let config_path = if let Some(path) = args.config {
        Some(path)
    } else {
        // Search order: 1. input_name.json, 2. config.json
        let mut auto_path = args.input.clone();
        auto_path.set_extension("json");
        if auto_path.exists() {
            Some(auto_path)
        } else {
            let shared_config = Path::new("config.json");
            if shared_config.exists() { Some(shared_config.to_path_buf()) } else { None }
        }
    };

    if let Some(path) = config_path {
        println!("Embedding config from {:?}...", path);
        let mut config_file = File::open(path).context("Failed to open config file")?;
        config_file.read_to_end(&mut config_data)?;
        let value: serde_json::Value = serde_json::from_slice(&config_data).context("Invalid JSON configuration")?;
        anyhow::ensure!(value.is_object(), "Configuration must be a JSON object");
    }

    // 6. Prepare and Compress data
    let compressed_snapshot = compress_data(&snapshot_data)?;
    let compressed_shader = if !shader_data.is_empty() { Some(compress_data(&shader_data)?) } else { None };
    let compressed_pokes = if !pokes_data.is_empty() { Some(compress_data(&pokes_data)?) } else { None };
    let compressed_config = if !config_data.is_empty() { Some(compress_data(&config_data)?) } else { None };

    let sizes = [compressed_snapshot.len(), compressed_shader.as_ref().map_or(0, Vec::len),
        compressed_pokes.as_ref().map_or(0, Vec::len), compressed_config.as_ref().map_or(0, Vec::len)];
    let mut footer = FOOTER_MAGIC.to_vec();
    for size in sizes {
        footer.extend_from_slice(&u32::try_from(size).context("Bundle section exceeds 4 GiB")?.to_le_bytes());
    }
    // Write a separate file so failure leaves an existing game intact.
    let directory = output_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if let Ok(output) = output_path.canonicalize() {
        anyhow::ensure!(output != args.input.canonicalize()? && output != runner.canonicalize()?,
            "Output would replace the snapshot or runner template");
    }
    let mut output_file = tempfile::NamedTempFile::new_in(directory)?;
    output_file.write_all(&runner_data)?;
    output_file.write_all(&compressed_snapshot)?;
    if let Some(v) = compressed_shader {
        output_file.write_all(&v)?;
    }
    if let Some(v) = compressed_pokes {
        output_file.write_all(&v)?;
    }
    if let Some(v) = compressed_config {
        output_file.write_all(&v)?;
    }
    output_file.write_all(&footer)?;

    #[cfg(unix)]
    {
        let mut perms = output_file.as_file().metadata()?.permissions();
        perms.set_mode(0o755);
        output_file.as_file().set_permissions(perms)?;
        println!("Set executable permissions on {:?}", output_path);
    }

    let size = output_file.as_file().metadata()?.len();
    output_file.persist(&output_path)?;
    println!("Successfully created {:?} (Total size: {} bytes)", output_path, size);

    Ok(())
}

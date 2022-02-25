use eyre::Result;
use std::io::{ Read, Write };
use std::process;
use clap::Parser;
use std::path::{ Path ,PathBuf };
use serde::{Deserialize, Serialize};
use subprocess::Exec;
use cacache::WriteOpts;
use sha2::{Sha256, Digest};

fn get_default_cache_dir() -> &'static str {
    let mut pb = PathBuf::from(home::home_dir().expect("Cannot operate without a home directory"));
    pb.push(".clawbang-cache");
    let f = pb.to_string_lossy().into_owned();
    Box::leak(f.into_boxed_str())
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Options {
    #[clap(short, long, parse(from_occurrences))]
    verbose: usize,

    #[clap(long, env="CLAWBANG_DIR", default_value=get_default_cache_dir())]
    cache_dir: PathBuf,

    #[clap(default_value="/dev/fd/0")]
    file: PathBuf,

    rest: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    output_id: String, // content ref of the output
    exit_code: i32,
}

struct Tee<Inner: std::io::Write> {
    accum: Vec<u8>,
    inner: Inner,
}

impl<Inner: std::io::Write> Tee<Inner> {
    pub(crate) fn new(inner: Inner) -> Self {
        Self {
            accum: Vec::new(),
            inner
        }
    }

    pub(crate) fn into_inner(self) -> (Vec<u8>, Inner) {
        (self.accum, self.inner)
    }
}

impl<Inner: std::io::Write> std::io::Write for Tee<Inner> {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, std::io::Error> {
        self.accum.extend(bytes);
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn main() -> Result<()> {
    // positional arguments check comes first: are we reading from a file or stdin?
    let opts = Options::parse();
    let mut file = std::fs::OpenOptions::new().read(true).open(opts.file)?;

    let mut input = Vec::new();
    file.read_to_end(&mut input)?;
    let source = String::from_utf8(input)?;
    let tempdir = tempfile::tempdir()?;
    let mut pb = PathBuf::from(tempdir.as_ref());

    let cache_key = get_key(&source);

    let metadata = cacache::metadata_sync(opts.cache_dir.as_path(), &cache_key)?;

    if let Some(metadata) = metadata {
        let cache_entry: CacheEntry = serde_json::from_value(metadata.metadata)?;

        if cache_entry.exit_code == 0 {
            pb.push("bin");
            cacache::copy_sync(opts.cache_dir.as_path(), &cache_key, pb.as_path())?;

            #[cfg(not(target_os = "windows"))]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(pb.as_path(), std::fs::Permissions::from_mode(0o755))?;
            }
        } else {
            let build_output = cacache::read_sync(opts.cache_dir.as_path(), cache_entry.output_id)?;
            std::io::stderr().write_all(&build_output[..])?;
            process::exit(cache_entry.exit_code);
        }
    } else {
        if opts.verbose < 1 {
            populate_cache(
                &cache_key,
                opts.cache_dir.as_path(),
                pb.as_path(),
                std::io::sink(),
                source.as_str()
            )?;
        } else {
            populate_cache(
                &cache_key,
                opts.cache_dir.as_path(),
                pb.as_path(),
                std::io::sink(),
                source.as_str()
            )?;
        }

        pb.push("target");
        pb.push("release");
        pb.push("bin");
    }


    let mut exec = Exec::cmd(&pb).cwd(std::env::current_dir()?);
    for arg in opts.rest {
        exec = exec.arg(arg);
    }

    std::process::exit(match exec.join()? {
        subprocess::ExitStatus::Exited(xs) => xs as i32,
        subprocess::ExitStatus::Signaled(xs) => xs as i32,
        subprocess::ExitStatus::Other(xs) => xs,
        subprocess::ExitStatus::Undetermined => -1,
    });
}

fn get_key(input: impl AsRef<str>) -> String {
    let mut hasher = Sha256::new();
    let bytes = input.as_ref().as_bytes();
    hasher.update(bytes);

    let hash_bytes = &hasher.finalize()[..];

    hex::encode(hash_bytes)
}

fn populate_cache(
    cache_key: &str,
    cache: impl AsRef<Path>,
    tempdir: impl AsRef<Path>,
    stdout: impl Write,
    source: &str
) -> Result<()> {
    let mut pb = PathBuf::from(tempdir.as_ref());
    let trimmed = if source.trim().starts_with("#!") {
        source[source.find("\n").unwrap() + 1..].trim()
    } else {
        source.trim()
    }; 

    let (frontmatter, rust_src) = if trimmed.starts_with("+++\n") {
        let offset = trimmed[4..].find("\n+++\n").ok_or_else(|| eyre::eyre!("Hit EOF before finding end of frontmatter delimeter, \"+++\"."))?;
        (&trimmed[4..offset + 4], &trimmed[offset + 9..])
    } else {
        (&trimmed[0..0], &trimmed[0..])
    };

    let mut frontmatter: toml::Value = toml::from_str(frontmatter)?;

    let tbl = frontmatter.as_table_mut().ok_or_else(|| eyre::eyre!("Expected frontmatter to contain valid TOML, but the top level is not a table"))?;
    let cargo_toml_pkg = tbl.entry("package").or_insert(toml::Value::Table(toml::map::Map::new())).as_table_mut().unwrap();
    cargo_toml_pkg.insert("name".to_string(), toml::Value::String("bin".to_string()));
    cargo_toml_pkg.insert("version".to_string(), toml::Value::String("0.0.1".to_string()));
    cargo_toml_pkg.insert("edition".to_string(), toml::Value::String("2021".to_string()));

    let cargo_toml = toml::to_string_pretty(&frontmatter)?;

    pb.push("Cargo.toml");
    {
        let mut cargo_toml_file = std::fs::OpenOptions::new().write(true).create(true).open(&pb)?;
        cargo_toml_file.write_all(cargo_toml.as_bytes())?;
    }
    pb.pop();

    pb.push("src");
    std::fs::create_dir(&pb)?;
    pb.push("main.rs");
    {
        let mut src_file = std::fs::OpenOptions::new().write(true).create(true).open(&pb)?;
        src_file.write_all(rust_src.as_bytes())?;
    }
    pb.pop();
    pb.pop();

    let mut popen = Exec::cmd("cargo")
        .arg("--color")
        .arg("always")
        .arg("build")
        .arg("--release")
        .stdout(subprocess::Redirection::Pipe)
        .stderr(subprocess::Redirection::Merge)
        .cwd(&tempdir)
        .popen()?;

    let mut out = Tee::new(stdout);
    while popen.poll().is_none() {
        if let Some(mut pstdout) = popen.stdout.as_mut() {
            std::io::copy(&mut pstdout, &mut out)?;
        }
    }

    let exit_code = match popen.exit_status() {
        Some(subprocess::ExitStatus::Exited(xs)) => xs as i32,
        Some(subprocess::ExitStatus::Signaled(xs)) => xs as i32,
        Some(subprocess::ExitStatus::Other(xs)) => xs,
        _ => 1
    };

    let (accum, _) = out.into_inner();

    let output_hash = cacache::write_hash_sync(&cache, accum)?;

    let build_metadata = CacheEntry {
        output_id: output_hash.to_string(),
        exit_code
    };

    pb.push("target");
    pb.push("release");
    pb.push("bin");

    let mut binary_file = std::fs::OpenOptions::new().read(true).open(&pb)?;

    let mut writer = WriteOpts::new()
        .algorithm(cacache::Algorithm::Sha256)
        .metadata(serde_json::to_value(build_metadata)?)
        .open_sync(&cache, cache_key)?;

    std::io::copy(&mut binary_file, &mut writer)?;

    writer.commit()?;

    Ok(())
}

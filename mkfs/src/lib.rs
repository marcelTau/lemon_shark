use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use filesystem::{BLOCK_SIZE, BlockDevice, BlockIndex, Filesystem};

pub const DEFAULT_SOURCE: &str = "rootfs";
pub const DEFAULT_OUTPUT: &str = "lemonfs.img";
pub const DEFAULT_BLOCKS: usize = 16 * 1024 * 1024 / BLOCK_SIZE;
pub const MAX_FILE_SIZE: u64 = 16 * BLOCK_SIZE as u64;

pub const USAGE: &str = "Usage: mkfs [OPTIONS]

Build a LemonFS image from a host directory.

Options:
    --source <DIR>    Source directory (default: rootfs)
    --output <FILE>   Output image (default: lemonfs.img)
    --blocks <COUNT>  Image size in 512-byte blocks (default: 32768)
    -h, --help        Show this help

The source directory's children become entries in the image root. LemonFS file
names are limited to 24 UTF-8 bytes and files are limited to 8192 bytes.
Symlinks and other non-regular entries are skipped.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub source: PathBuf,
    pub output: PathBuf,
    pub total_blocks: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: DEFAULT_SOURCE.into(),
            output: DEFAULT_OUTPUT.into(),
            total_blocks: DEFAULT_BLOCKS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Build(Config),
    Help,
}

impl Command {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, BuildError> {
        let mut config = Config::default();
        let mut source_seen = false;
        let mut output_seen = false;
        let mut blocks_seen = false;
        let mut args = args.into_iter();

        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("-h" | "--help") => return Ok(Self::Help),
                Some("--source") => {
                    reject_duplicate(&mut source_seen, "--source")?;
                    config.source = next_value(&mut args, "--source")?.into();
                }
                Some("--output") => {
                    reject_duplicate(&mut output_seen, "--output")?;
                    config.output = next_value(&mut args, "--output")?.into();
                }
                Some("--blocks") => {
                    reject_duplicate(&mut blocks_seen, "--blocks")?;
                    let value = next_value(&mut args, "--blocks")?;
                    let value = value
                        .to_str()
                        .ok_or_else(|| BuildError::new("--blocks must be valid UTF-8"))?;
                    config.total_blocks = value
                        .parse()
                        .map_err(|_| BuildError::new(format!("invalid block count {value:?}")))?;
                    if config.total_blocks == 0 {
                        return Err(BuildError::new("--blocks must be greater than zero"));
                    }
                }
                Some(argument) => {
                    return Err(BuildError::new(format!("unknown argument {argument:?}")));
                }
                None => return Err(BuildError::new("arguments must be valid UTF-8")),
            }
        }

        Ok(Self::Build(config))
    }
}

fn reject_duplicate(seen: &mut bool, option: &str) -> Result<(), BuildError> {
    if *seen {
        return Err(BuildError::new(format!(
            "{option} may only be specified once"
        )));
    }
    *seen = true;
    Ok(())
}

fn next_value(
    args: &mut impl Iterator<Item = OsString>,
    option: &str,
) -> Result<OsString, BuildError> {
    args.next()
        .ok_or_else(|| BuildError::new(format!("missing value for {option}")))
}

#[derive(Debug)]
pub struct BuildError {
    message: String,
}

impl BuildError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BuildError {}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub directories: usize,
    pub files: usize,
    pub skipped: Vec<PathBuf>,
}

struct FileBlockDevice {
    file: File,
    total_blocks: usize,
}

impl FileBlockDevice {
    fn create_new(path: &Path, total_blocks: usize) -> Result<Self, BuildError> {
        let image_bytes = total_blocks
            .checked_mul(BLOCK_SIZE)
            .ok_or_else(|| BuildError::new("image size overflows usize"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| io_error("create temporary image", path, error))?;
        if let Err(error) = file.set_len(image_bytes as u64) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(io_error("size temporary image", path, error));
        }
        Ok(Self { file, total_blocks })
    }

    fn open(path: &Path) -> Result<Self, BuildError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| io_error("open formatted image", path, error))?;
        let byte_len = file
            .metadata()
            .map_err(|error| io_error("inspect formatted image", path, error))?
            .len();
        if byte_len % BLOCK_SIZE as u64 != 0 {
            return Err(BuildError::new(format!(
                "image {} is not a whole number of blocks",
                path.display()
            )));
        }
        let total_blocks = usize::try_from(byte_len / BLOCK_SIZE as u64)
            .map_err(|_| BuildError::new("image has too many blocks for this host"))?;
        Ok(Self { file, total_blocks })
    }
}

impl BlockDevice for FileBlockDevice {
    fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
        self.file
            .seek(SeekFrom::Start(
                block_idx.inner() as u64 * BLOCK_SIZE as u64,
            ))
            .expect("failed to seek while reading image");
        self.file
            .read_exact(buf)
            .expect("failed to read filesystem block");
    }

    fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
        self.file
            .seek(SeekFrom::Start(
                block_idx.inner() as u64 * BLOCK_SIZE as u64,
            ))
            .expect("failed to seek while writing image");
        self.file
            .write_all(data)
            .expect("failed to write filesystem block");
    }

    fn total_blocks(&mut self) -> usize {
        self.total_blocks
    }
}

struct TemporaryImage {
    path: PathBuf,
    keep: bool,
}

impl Drop for TemporaryImage {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn build_image(config: &Config) -> Result<ImportSummary, BuildError> {
    let metadata = fs::symlink_metadata(&config.source)
        .map_err(|error| io_error("inspect source directory", &config.source, error))?;
    if !metadata.file_type().is_dir() {
        return Err(BuildError::new(format!(
            "source {} is not a directory",
            config.source.display()
        )));
    }
    validate_output_location(&config.source, &config.output)?;

    if u32::try_from(config.total_blocks).is_err() {
        return Err(BuildError::new(format!(
            "block count {} exceeds the LemonFS limit",
            config.total_blocks
        )));
    }

    let (temporary_path, device) = create_temporary_image(&config.output, config.total_blocks)?;
    let mut temporary = TemporaryImage {
        path: temporary_path,
        keep: false,
    };

    Filesystem::format(device)
        .map_err(|error| BuildError::new(format!("format image: {error}")))?;
    let device = FileBlockDevice::open(&temporary.path)?;
    let mut filesystem = Filesystem::new(device)
        .map_err(|error| BuildError::new(format!("mount new image: {error}")))?;

    let mut summary = ImportSummary::default();
    import_directory(&mut filesystem, &config.source, "/", &mut summary)?;
    filesystem.flush();
    drop(filesystem);

    fs::rename(&temporary.path, &config.output)
        .map_err(|error| io_error("replace output image", &config.output, error))?;
    temporary.keep = true;

    Ok(summary)
}

fn validate_output_location(source: &Path, output: &Path) -> Result<(), BuildError> {
    let source = fs::canonicalize(source)
        .map_err(|error| io_error("resolve source directory", source, error))?;
    let output_name = output
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| BuildError::new("output must name an image file"))?;
    let output_parent = output.parent().unwrap_or_else(|| Path::new("."));
    let output_parent = fs::canonicalize(output_parent)
        .map_err(|error| io_error("resolve output directory", output_parent, error))?;
    let output = output_parent.join(output_name);

    if output.starts_with(&source) {
        return Err(BuildError::new(format!(
            "output image {} cannot be inside source directory {}",
            output.display(),
            source.display()
        )));
    }

    Ok(())
}

fn create_temporary_image(
    output: &Path,
    total_blocks: usize,
) -> Result<(PathBuf, FileBlockDevice), BuildError> {
    let file_name = output
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| BuildError::new("output must name an image file"))?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));

    for attempt in 0..100 {
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".tmp-{}-{attempt}", std::process::id()));
        let path = parent.join(temporary_name);
        match FileBlockDevice::create_new(&path, total_blocks) {
            Ok(device) => return Ok((path, device)),
            Err(_error) if path.exists() => continue,
            Err(error) => return Err(error),
        }
    }

    Err(BuildError::new(format!(
        "could not allocate a temporary image beside {}",
        output.display()
    )))
}

fn import_directory(
    filesystem: &mut Filesystem<FileBlockDevice>,
    host_directory: &Path,
    lemon_directory: &str,
    summary: &mut ImportSummary,
) -> Result<(), BuildError> {
    let reader = fs::read_dir(host_directory)
        .map_err(|error| io_error("read source directory", host_directory, error))?;
    let mut entries = reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("read entry in source directory", host_directory, error))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let host_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("inspect source entry", &host_path, error))?;
        if !file_type.is_dir() && !file_type.is_file() {
            summary.skipped.push(host_path);
            continue;
        }

        let entry_name = entry.file_name();
        let name = utf8_name(&entry_name, &host_path)?;
        let lemon_path = child_path(lemon_directory, name);

        if file_type.is_dir() {
            filesystem.mkdir(&lemon_path).map_err(|error| {
                BuildError::new(format!(
                    "create directory {} from {}: {error}",
                    lemon_path,
                    host_path.display()
                ))
            })?;
            summary.directories += 1;
            import_directory(filesystem, &host_path, &lemon_path, summary)?;
        } else {
            let metadata = entry
                .metadata()
                .map_err(|error| io_error("inspect source file", &host_path, error))?;
            if metadata.len() > MAX_FILE_SIZE {
                return Err(BuildError::new(format!(
                    "source file {} is {} bytes; LemonFS files are limited to {} bytes",
                    host_path.display(),
                    metadata.len(),
                    MAX_FILE_SIZE
                )));
            }
            let contents = fs::read(&host_path)
                .map_err(|error| io_error("read source file", &host_path, error))?;
            filesystem.create_file(&lemon_path).map_err(|error| {
                BuildError::new(format!(
                    "create file {} from {}: {error}",
                    lemon_path,
                    host_path.display()
                ))
            })?;
            filesystem
                .write_to_file(&lemon_path, &contents)
                .map_err(|error| {
                    BuildError::new(format!(
                        "write file {} from {}: {error}",
                        lemon_path,
                        host_path.display()
                    ))
                })?;
            summary.files += 1;
        }
    }

    Ok(())
}

fn utf8_name<'a>(name: &'a OsStr, path: &Path) -> Result<&'a str, BuildError> {
    name.to_str().ok_or_else(|| {
        BuildError::new(format!(
            "source entry {} does not have a UTF-8 name",
            path.display()
        ))
    })
}

fn child_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> BuildError {
    BuildError::new(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_BLOCKS: usize = 1024;
    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("lemon-shark-mkfs-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn join(&self, path: impl AsRef<Path>) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn strings<'a>(args: &'a [&str]) -> impl Iterator<Item = OsString> + 'a {
        args.iter().map(OsString::from)
    }

    #[test]
    fn parses_defaults() {
        assert_eq!(
            Command::parse(std::iter::empty()).unwrap(),
            Command::Build(Config::default())
        );
    }

    #[test]
    fn parses_named_options() {
        assert_eq!(
            Command::parse(strings(&[
                "--source", "input", "--output", "disk.img", "--blocks", "2048",
            ]))
            .unwrap(),
            Command::Build(Config {
                source: "input".into(),
                output: "disk.img".into(),
                total_blocks: 2048,
            })
        );
    }

    #[test]
    fn parses_help_and_rejects_invalid_arguments() {
        assert_eq!(Command::parse(strings(&["--help"])).unwrap(), Command::Help);
        assert!(Command::parse(strings(&["--wat"])).is_err());
        assert!(Command::parse(strings(&["--source"])).is_err());
        assert!(Command::parse(strings(&["--blocks", "zero"])).is_err());
        assert!(Command::parse(strings(&["--blocks", "0"])).is_err());
        assert!(Command::parse(strings(&["--output", "one", "--output", "two"])).is_err());
    }

    #[test]
    fn imports_nested_tree_and_file_contents() {
        let temp = TempDir::new();
        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join(".hidden"), b"secret").unwrap();
        fs::write(source.join("binary.dat"), [0, 159, 146, 150]).unwrap();
        let long_contents = "lemon".repeat(140);
        fs::write(source.join("nested/note.txt"), &long_contents).unwrap();

        let output = temp.join("result.img");
        let summary = build_image(&Config {
            source,
            output: output.clone(),
            total_blocks: TEST_BLOCKS,
        })
        .unwrap();

        assert_eq!(summary.directories, 2);
        assert_eq!(summary.files, 3);
        assert!(summary.skipped.is_empty());

        let mut filesystem = Filesystem::new(FileBlockDevice::open(&output).unwrap()).unwrap();
        assert_eq!(filesystem.read_file("/.hidden").unwrap(), "secret");
        assert_eq!(
            filesystem.read_file("/nested/note.txt").unwrap(),
            long_contents
        );
        let mut listing = String::new();
        filesystem.dump_dir("/empty", &mut listing).unwrap();
        assert!(listing.contains("./"));
        assert!(listing.contains("../"));

        let second_output = temp.join("second.img");
        build_image(&Config {
            source: temp.join("source"),
            output: second_output.clone(),
            total_blocks: TEST_BLOCKS,
        })
        .unwrap();
        assert_eq!(fs::read(output).unwrap(), fs::read(second_output).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks_without_following_them() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("real.txt"), b"real").unwrap();
        symlink("real.txt", source.join("link.txt")).unwrap();

        let output = temp.join("result.img");
        let summary = build_image(&Config {
            source,
            output: output.clone(),
            total_blocks: TEST_BLOCKS,
        })
        .unwrap();

        assert_eq!(summary.files, 1);
        assert_eq!(summary.skipped.len(), 1);
        let mut filesystem = Filesystem::new(FileBlockDevice::open(&output).unwrap()).unwrap();
        assert_eq!(filesystem.read_file("/real.txt").unwrap(), "real");
        assert!(filesystem.read_file("/link.txt").is_err());
    }

    #[test]
    fn failed_import_preserves_existing_output() {
        let temp = TempDir::new();
        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("too-large"),
            vec![b'x'; MAX_FILE_SIZE as usize + 1],
        )
        .unwrap();
        let output = temp.join("result.img");
        fs::write(&output, b"keep me").unwrap();

        let error = build_image(&Config {
            source,
            output: output.clone(),
            total_blocks: TEST_BLOCKS,
        })
        .unwrap_err();

        assert!(error.to_string().contains("limited"));
        assert_eq!(fs::read(output).unwrap(), b"keep me");
    }

    #[test]
    fn rejects_missing_source_and_long_names() {
        let temp = TempDir::new();
        let missing = build_image(&Config {
            source: temp.join("missing"),
            output: temp.join("missing.img"),
            total_blocks: TEST_BLOCKS,
        });
        assert!(missing.is_err());

        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("this-name-is-more-than-24-bytes"), b"x").unwrap();
        let error = build_image(&Config {
            source,
            output: temp.join("long-name.img"),
            total_blocks: TEST_BLOCKS,
        })
        .unwrap_err();
        assert!(error.to_string().contains("NameTooLong"));
    }

    #[test]
    fn rejects_output_inside_source_tree() {
        let temp = TempDir::new();
        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        let error = build_image(&Config {
            output: source.join("lemonfs.img"),
            source,
            total_blocks: TEST_BLOCKS,
        })
        .unwrap_err();
        assert!(error.to_string().contains("cannot be inside source"));
    }

    #[test]
    fn rejects_image_that_is_too_small() {
        let temp = TempDir::new();
        let source = temp.join("source");
        fs::create_dir(&source).unwrap();
        let error = build_image(&Config {
            source,
            output: temp.join("small.img"),
            total_blocks: 1,
        })
        .unwrap_err();
        assert!(error.to_string().contains("DeviceTooSmall"));
    }
}

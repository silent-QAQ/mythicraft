use mythicraft_api::DataVersionRange;
use mythicraft_observability::{LatencySummary, PerformanceMetadata, PerformanceReport};
use mythicraft_persistence::SaveStore;
use mythicraft_vanilla_data::TARGET_DATA_VERSION;
use mythicraft_world::{
    inspect_world_directory, ChunkInspectionIssueKind, ChunkNbtSchema, WorldFileIssueKind,
    WorldInspectionLimits, WorldInspectionSummary,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureManifest {
    schema_version: u32,
    fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEntry {
    path: String,
    sha256: String,
    source: String,
    upstream_version: String,
    expected_result: String,
    license_status: String,
    redistributable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: u32,
    package_version: String,
    target: String,
    files: Vec<ReleaseFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseFile {
    path: String,
    sha256: String,
    role: String,
    source: String,
    license_status: String,
    redistributable: bool,
}

const REQUIRED_RELEASE_ROLES: &[&str] = &[
    "server_binary",
    "example_config",
    "map_checker",
    "config_migrator",
    "client_mod",
    "resource_manifest",
    "compatibility_matrix",
    "runbook",
    "known_limitations",
];

const REQUIRED_PERFORMANCE_SCENARIOS: &[&str] = &[
    "static_player_movement",
    "entity_dense",
    "high_frequency_skills",
    "multiplayer_visibility_broadcast",
    "bulk_chunk_send",
    "economy_concurrency",
    "ui_audio_high_frequency",
];

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("fixture-verify") => {
            let root = arguments.next().unwrap_or_else(|| "fixtures".into());
            ensure_no_extra_arguments(arguments)?;
            verify_fixtures(Path::new(&root))
        }
        Some("release-scan") => {
            let roots: Vec<_> = arguments.map(PathBuf::from).collect();
            if roots.is_empty() {
                return Err("release-scan requires at least one package root".into());
            }
            scan_release(&roots)
        }
        Some("release-manifest-verify") => {
            let staging = required_argument(&mut arguments, "staging root")?;
            let manifest = required_argument(&mut arguments, "release manifest")?;
            ensure_no_extra_arguments(arguments)?;
            verify_release_manifest(Path::new(&staging), Path::new(&manifest))
        }
        Some("world-inspect") => {
            let root = required_argument(&mut arguments, "world root")?;
            let known_top_level_tags = arguments.collect::<Vec<_>>();
            let report = inspect_world_json(Path::new(&root), known_top_level_tags)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("save-inspect") => {
            let root = required_argument(&mut arguments, "save root")?;
            let player_id = required_argument(&mut arguments, "player id")?;
            ensure_no_extra_arguments(arguments)?;
            let loaded = SaveStore::open(root)?.load(&player_id)?;
            println!("{}", serde_json::to_string_pretty(&loaded.state)?);
            eprintln!(
                "revision={} recovered_from={:?}",
                loaded.revision, loaded.recovered_from
            );
            Ok(())
        }
        Some("save-backup") => {
            let root = required_argument(&mut arguments, "save root")?;
            let player_id = required_argument(&mut arguments, "player id")?;
            let label = required_argument(&mut arguments, "backup label")?;
            ensure_no_extra_arguments(arguments)?;
            let path = SaveStore::open(root)?.create_backup(&player_id, &label)?;
            println!("{}", path.display());
            Ok(())
        }
        Some("save-restore") => {
            let root = required_argument(&mut arguments, "save root")?;
            let backup = required_argument(&mut arguments, "backup path")?;
            ensure_no_extra_arguments(arguments)?;
            let revision = SaveStore::open(root)?.restore_backup(Path::new(&backup))?;
            println!("restored_revision={revision}");
            Ok(())
        }
        Some("perf-report") => {
            let input = required_argument(&mut arguments, "performance input")?;
            let output = required_argument(&mut arguments, "performance output")?;
            ensure_no_extra_arguments(arguments)?;
            write_performance_report(Path::new(&input), Path::new(&output))
        }
        Some("perf-suite-verify") => {
            let reports = required_argument(&mut arguments, "performance report directory")?;
            ensure_no_extra_arguments(arguments)?;
            verify_performance_suite(Path::new(&reports))
        }
        _ => {
            print_usage();
            Err("unknown or missing command".into())
        }
    }
}

fn required_argument(
    arguments: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn Error>> {
    arguments
        .next()
        .ok_or_else(|| format!("missing {name}").into())
}

fn ensure_no_extra_arguments(
    mut arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(argument) = arguments.next() {
        return Err(format!("unexpected argument: {argument}").into());
    }
    Ok(())
}

fn print_usage() {
    eprintln!("mythicraft-tools commands:");
    eprintln!("  fixture-verify [fixtures-root]");
    eprintln!("  release-scan <package-root> [package-root ...]");
    eprintln!("  release-manifest-verify <staging-root> <manifest.json>");
    eprintln!("  world-inspect <world-root> [known-top-level-tag ...]");
    eprintln!("  save-inspect <save-root> <player-id>");
    eprintln!("  save-backup <save-root> <player-id> <label>");
    eprintln!("  save-restore <save-root> <backup-path>");
    eprintln!("  perf-report <input.json> <output.json>");
    eprintln!("  perf-suite-verify <report-directory>");
}

fn inspect_world_json(
    world_root: &Path,
    known_top_level_tags: Vec<String>,
) -> Result<Value, Box<dyn Error>> {
    let schema = ChunkNbtSchema::new(
        DataVersionRange {
            minimum: TARGET_DATA_VERSION,
            maximum: TARGET_DATA_VERSION,
        },
        known_top_level_tags,
    );
    let summary = inspect_world_directory(world_root, &schema, WorldInspectionLimits::default())?;
    Ok(world_summary_json(&summary, &schema))
}

fn world_summary_json(summary: &WorldInspectionSummary, schema: &ChunkNbtSchema) -> Value {
    json!({
        "schema_version": 1,
        "target_data_version": {
            "minimum": schema.data_version.minimum,
            "maximum": schema.data_version.maximum,
        },
        "known_top_level_tags": schema.known_top_level_tags.iter().collect::<Vec<_>>(),
        "level_dat": {
            "data_version": summary.level_dat.data_version,
            "supported": summary.level_dat.supported,
            "compressed_bytes": summary.level_dat.compressed_bytes,
            "decompressed_bytes": summary.level_dat.decompressed_bytes,
        },
        "region_count": summary.region_count,
        "present_chunk_count": summary.present_chunk_count,
        "inspected_chunk_count": summary.inspected_chunk_count,
        "coordinate_bounds": summary.coordinate_bounds.map(|bounds| json!({
            "minimum_x": bounds.minimum_x,
            "maximum_x": bounds.maximum_x,
            "minimum_z": bounds.minimum_z,
            "maximum_z": bounds.maximum_z,
        })),
        "data_versions": summary.data_versions,
        "unknown_top_level_tags": summary.unknown_top_level_tags.iter().map(|tag| json!({
            "name": tag.name,
            "occurrences": tag.occurrences,
        })).collect::<Vec<_>>(),
        "total_region_file_bytes": summary.total_region_file_bytes,
        "total_decompressed_chunk_bytes": summary.total_decompressed_chunk_bytes,
        "issues": summary.issues.iter().map(world_file_issue_json).collect::<Vec<_>>(),
        "regions": summary.regions.iter().map(world_region_json).collect::<Vec<_>>(),
    })
}

fn world_region_json(region: &mythicraft_world::WorldRegionInspection) -> Value {
    json!({
        "relative_path": region.relative_path,
        "region_x": region.region_x,
        "region_z": region.region_z,
        "file_bytes": region.file_bytes,
        "present_chunk_count": region.summary.region.present_chunk_count,
        "inspected_chunk_count": region.summary.chunks.len(),
        "data_versions": region.summary.data_versions,
        "unknown_top_level_tags": region.summary.unknown_top_level_tags.iter().map(|tag| json!({
            "name": tag.name,
            "occurrences": tag.occurrences,
        })).collect::<Vec<_>>(),
        "chunks": region.summary.chunks.iter().map(chunk_inspection_json).collect::<Vec<_>>(),
        "issues": region.summary.issues.iter().map(|issue| json!({
            "index": issue.index,
            "local_x": issue.local_x,
            "local_z": issue.local_z,
            "detail": chunk_issue_json(&issue.kind),
        })).collect::<Vec<_>>(),
    })
}

fn chunk_inspection_json(chunk: &mythicraft_world::ChunkInspection) -> Value {
    json!({
        "index": chunk.index,
        "local_x": chunk.local_x,
        "local_z": chunk.local_z,
        "data_version": chunk.data_version,
        "compressed_bytes": chunk.compressed_bytes,
        "decompressed_bytes": chunk.decompressed_bytes,
        "sections": chunk.sections.iter().map(section_inspection_json).collect::<Vec<_>>(),
        "heightmaps": chunk.heightmaps.iter().map(heightmap_inspection_json).collect::<Vec<_>>(),
    })
}

fn section_inspection_json(section: &mythicraft_world::SectionInspectionSummary) -> Value {
    json!({
        "y": section.y,
        "has_block_states": section.has_block_states,
        "palette_entry_count": section.palette_entry_count,
        "bits_per_entry": section.bits_per_entry,
        "packed_word_count": section.packed_word_count,
        "decoded_block_count": section.decoded_block_count,
        "homogeneous": section.homogeneous,
        "block_light_bytes": section.block_light_bytes,
        "sky_light_bytes": section.sky_light_bytes,
    })
}

fn heightmap_inspection_json(heightmap: &mythicraft_world::HeightmapInspectionSummary) -> Value {
    json!({
        "name": heightmap.name,
        "packed_word_count": heightmap.packed_word_count,
        "decoded_column_count": heightmap.decoded_column_count,
        "minimum_value": heightmap.minimum_value,
        "maximum_value": heightmap.maximum_value,
    })
}

fn world_file_issue_json(issue: &mythicraft_world::WorldFileIssue) -> Value {
    let detail = match &issue.kind {
        WorldFileIssueKind::InvalidRegionFileName => json!({
            "code": "invalid_region_file_name",
        }),
        WorldFileIssueKind::SymlinkNotAllowed => json!({
            "code": "symlink_not_allowed",
        }),
        WorldFileIssueKind::FileTooLarge {
            actual_bytes,
            max_bytes,
        } => json!({
            "code": "file_too_large",
            "actual_bytes": actual_bytes,
            "max_bytes": max_bytes,
        }),
        WorldFileIssueKind::MetadataFailed { message } => json!({
            "code": "metadata_failed",
            "message": message,
        }),
        WorldFileIssueKind::ReadFailed { message } => json!({
            "code": "read_failed",
            "message": message,
        }),
        WorldFileIssueKind::RegionInspectionFailed { message } => json!({
            "code": "region_inspection_failed",
            "message": message,
        }),
    };
    json!({
        "relative_path": issue.relative_path,
        "detail": detail,
    })
}

fn chunk_issue_json(issue: &ChunkInspectionIssueKind) -> Value {
    match issue {
        ChunkInspectionIssueKind::ChunkRead(error) => json!({
            "code": "chunk_read_failed",
            "message": error.to_string(),
        }),
        ChunkInspectionIssueKind::Nbt(error) => json!({
            "code": "nbt_invalid",
            "message": error.to_string(),
        }),
        ChunkInspectionIssueKind::Sections(error) => json!({
            "code": "section_invalid",
            "message": error.to_string(),
        }),
        ChunkInspectionIssueKind::Heightmaps(error) => json!({
            "code": "heightmap_invalid",
            "message": error.to_string(),
        }),
        ChunkInspectionIssueKind::RootNotCompound => json!({
            "code": "root_not_compound",
        }),
        ChunkInspectionIssueKind::MissingDataVersion => json!({
            "code": "missing_data_version",
        }),
        ChunkInspectionIssueKind::InvalidDataVersionType => json!({
            "code": "invalid_data_version_type",
        }),
        ChunkInspectionIssueKind::UnsupportedDataVersion {
            actual,
            minimum,
            maximum,
        } => json!({
            "code": "unsupported_data_version",
            "actual": actual,
            "minimum": minimum,
            "maximum": maximum,
        }),
    }
}

fn verify_fixtures(root: &Path) -> Result<(), Box<dyn Error>> {
    let manifest_path = root.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest: FixtureManifest = serde_json::from_slice(strip_utf8_bom(&manifest_bytes))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported fixture manifest schema {}",
            manifest.schema_version
        )
        .into());
    }
    let mut declared = BTreeSet::new();
    for fixture in &manifest.fixtures {
        validate_relative_path(&fixture.path)?;
        if !declared.insert(fixture.path.clone()) {
            return Err(format!("duplicate fixture entry: {}", fixture.path).into());
        }
        if fixture.source.trim().is_empty()
            || fixture.upstream_version.trim().is_empty()
            || fixture.expected_result.trim().is_empty()
            || fixture.license_status.trim().is_empty()
        {
            return Err(format!("fixture {} has incomplete provenance", fixture.path).into());
        }
        if !fixture.redistributable {
            return Err(format!("fixture {} is marked non-redistributable", fixture.path).into());
        }
        let path = root.join(&fixture.path);
        let bytes = fs::read(&path)?;
        let actual = hex::encode(Sha256::digest(bytes));
        if actual != fixture.sha256 {
            return Err(format!(
                "hash mismatch for {}: expected {}, actual {actual}",
                fixture.path, fixture.sha256
            )
            .into());
        }
    }
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path() == manifest_path
            || matches!(
                entry.path().file_name().and_then(|name| name.to_str()),
                Some("README.md" | "CHANGELOG.md")
            )
        {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if !declared.contains(&relative) {
            return Err(format!("fixture is not declared in manifest: {relative}").into());
        }
    }
    println!("verified {} fixtures", manifest.fixtures.len());
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformanceInput {
    metadata: PerformanceMetadata,
    tick_samples_ms: Vec<f64>,
    memory_peak_bytes: u64,
    network_bytes_per_second: u64,
    blocking_tick_io_events: u64,
}

fn write_performance_report(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let input_bytes = fs::read(input)?;
    let input: PerformanceInput = serde_json::from_slice(strip_utf8_bom(&input_bytes))?;
    let report = PerformanceReport {
        metadata: input.metadata,
        tick_latency: LatencySummary::from_milliseconds(&input.tick_samples_ms)?,
        memory_peak_bytes: input.memory_peak_bytes,
        network_bytes_per_second: input.network_bytes_per_second,
        blocking_tick_io_events: input.blocking_tick_io_events,
    };
    report.write_json(output)?;
    println!(
        "samples={} p50_ms={} p95_ms={} p99_ms={} gate={}",
        report.tick_latency.samples,
        report.tick_latency.p50_ms,
        report.tick_latency.p95_ms,
        report.tick_latency.p99_ms,
        if report.meets_development_gate() {
            "pass"
        } else {
            "fail"
        }
    );
    if report.meets_development_gate() {
        Ok(())
    } else {
        Err("tick latency gate failed".into())
    }
}

fn verify_performance_suite(root: &Path) -> Result<(), Box<dyn Error>> {
    let mut scenarios = BTreeSet::new();
    let mut report_count = 0_usize;
    for entry in WalkDir::new(root)
        .min_depth(1)
        .max_depth(1)
        .follow_links(false)
    {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "performance report may not be a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        let report: PerformanceReport = serde_json::from_slice(strip_utf8_bom(&bytes))?;
        report.validate()?;
        if !report.meets_development_gate() {
            return Err(format!(
                "performance gate failed for scenario {}",
                report.metadata.scenario
            )
            .into());
        }
        if !scenarios.insert(report.metadata.scenario.clone()) {
            return Err(format!(
                "duplicate performance scenario: {}",
                report.metadata.scenario
            )
            .into());
        }
        report_count += 1;
    }
    for scenario in REQUIRED_PERFORMANCE_SCENARIOS {
        if !scenarios.contains(*scenario) {
            return Err(format!("performance suite is missing scenario: {scenario}").into());
        }
    }
    println!("verified {report_count} performance reports across all required scenarios");
    Ok(())
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)
}

fn verify_release_manifest(staging: &Path, manifest_path: &Path) -> Result<(), Box<dyn Error>> {
    let manifest_bytes = fs::read(manifest_path)?;
    let manifest: ReleaseManifest = serde_json::from_slice(strip_utf8_bom(&manifest_bytes))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported release manifest schema {}",
            manifest.schema_version
        )
        .into());
    }
    if manifest.package_version.trim().is_empty() || manifest.target.trim().is_empty() {
        return Err("release manifest package_version and target are required".into());
    }

    let mut declared_paths = BTreeSet::new();
    let mut declared_roles = BTreeSet::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        if !declared_paths.insert(file.path.clone()) {
            return Err(format!("duplicate release file: {}", file.path).into());
        }
        if file.role.trim().is_empty()
            || file.source.trim().is_empty()
            || file.license_status.trim().is_empty()
        {
            return Err(format!("release file {} has incomplete provenance", file.path).into());
        }
        if !file.redistributable {
            return Err(format!("release file {} is not redistributable", file.path).into());
        }
        declared_roles.insert(file.role.as_str());
        let path = staging.join(&file.path);
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(format!("release file may not be a symlink: {}", file.path).into());
        }
        let actual = hash_file(&path)?;
        if actual != file.sha256 {
            return Err(format!(
                "release hash mismatch for {}: expected {}, actual {actual}",
                file.path, file.sha256
            )
            .into());
        }
    }

    for role in REQUIRED_RELEASE_ROLES {
        if !declared_roles.contains(role) {
            return Err(format!("release manifest is missing required role: {role}").into());
        }
    }

    let manifest_canonical = fs::canonicalize(manifest_path)?;
    for entry in WalkDir::new(staging).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "release package contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if fs::canonicalize(entry.path())? == manifest_canonical {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(staging)?
            .to_string_lossy()
            .replace('\\', "/");
        if !declared_paths.contains(&relative) {
            return Err(format!("release file is not declared in manifest: {relative}").into());
        }
    }
    scan_release(&[staging.to_path_buf()])?;
    println!(
        "verified release {} for {} with {} files",
        manifest.package_version,
        manifest.target,
        manifest.files.len()
    );
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, Box<dyn Error>> {
    Ok(hex::encode(Sha256::digest(fs::read(path)?)))
}

fn validate_relative_path(path: &str) -> Result<(), Box<dyn Error>> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("fixture path escapes root: {}", path.display()).into());
    }
    Ok(())
}

fn scan_release(roots: &[PathBuf]) -> Result<(), Box<dyn Error>> {
    let mut scanned = 0_u64;
    let mut violations = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_symlink() {
                violations.push(format!(
                    "symlinks are forbidden in release packages: {}",
                    entry.path().display()
                ));
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            scanned += 1;
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "jar" | "class") {
                violations.push(format!("forbidden Java binary: {}", path.display()));
            }
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("level.dat"))
            {
                violations.push(format!(
                    "potential Mojang world asset requires explicit license review: {}",
                    path.display()
                ));
            }
        }
    }
    if violations.is_empty() {
        println!("scanned {scanned} release files; no forbidden distribution files found");
        Ok(())
    } else {
        Err(violations.join("\n").into())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hash_file, inspect_world_json, strip_utf8_bom, verify_performance_suite,
        verify_release_manifest, PerformanceMetadata, PerformanceReport,
        REQUIRED_PERFORMANCE_SCENARIOS, REQUIRED_RELEASE_ROLES,
    };
    use flate2::{write::GzEncoder, Compression};
    use mythicraft_observability::LatencySummary;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mythicraft-tools-{name}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn strips_optional_utf8_bom() {
        assert_eq!(strip_utf8_bom(b"\xef\xbb\xbf{}"), b"{}");
        assert_eq!(strip_utf8_bom(b"{}"), b"{}");
    }

    #[test]
    fn world_inspect_emits_stable_machine_readable_summary() {
        let root = temp_dir("world-inspect");
        let mut nbt = vec![10, 0, 0, 10, 0, 4, b'D', b'a', b't', b'a'];
        nbt.extend_from_slice(&[
            3, 0, 11, b'D', b'a', b't', b'a', b'V', b'e', b'r', b's', b'i', b'o', b'n', 0, 0, 0x13,
            0x27, 0, 0,
        ]);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&nbt)
            .expect("compress synthetic level.dat");
        fs::write(
            root.join("level.dat"),
            encoder.finish().expect("finish level.dat compression"),
        )
        .expect("write level.dat");

        let first = inspect_world_json(&root, vec!["sections".into(), "Heightmaps".into()])
            .expect("inspect synthetic world");
        let second = inspect_world_json(&root, vec!["sections".into(), "Heightmaps".into()])
            .expect("repeat synthetic world inspection");
        assert_eq!(first, second);
        assert_eq!(first["schema_version"], 1);
        assert_eq!(first["level_dat"]["data_version"], 4903);
        assert_eq!(first["level_dat"]["supported"], true);
        assert_eq!(first["region_count"], 0);
        assert_eq!(
            first["known_top_level_tags"],
            json!(["Heightmaps", "sections"])
        );
        fs::remove_dir_all(root).expect("cleanup world inspection fixture");
    }

    #[test]
    fn verifies_complete_release_manifest_and_rejects_extra_files() {
        let root = temp_dir("release");
        let staging = root.join("staging");
        fs::create_dir_all(&staging).expect("create staging");
        let mut files = Vec::new();
        for role in REQUIRED_RELEASE_ROLES {
            let relative = format!("{role}.txt");
            let path = staging.join(&relative);
            fs::write(&path, format!("synthetic {role}")).expect("write package file");
            files.push(json!({
                "path": relative,
                "sha256": hash_file(&path).expect("hash"),
                "role": role,
                "source": "project build",
                "license_status": "repository license",
                "redistributable": true
            }));
        }
        let manifest_path = root.join("release-manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": 1,
                "package_version": "0.1.0-preview.1",
                "target": "x86_64-unknown-linux-gnu",
                "files": files
            }))
            .expect("encode manifest"),
        )
        .expect("write manifest");
        verify_release_manifest(&staging, &manifest_path).expect("verify package");
        fs::write(staging.join("undeclared.txt"), "unexpected").expect("write extra file");
        assert!(verify_release_manifest(&staging, &manifest_path).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn verifies_all_required_performance_scenarios() {
        let root = temp_dir("performance");
        for scenario in REQUIRED_PERFORMANCE_SCENARIOS {
            PerformanceReport {
                metadata: PerformanceMetadata {
                    schema_version: 1,
                    scenario: (*scenario).into(),
                    machine: "test-host".into(),
                    operating_system: "test-os".into(),
                    rust_profile: "release".into(),
                    target: "test-target".into(),
                    minecraft_version: "test-version".into(),
                    map_hash: "00".into(),
                    config_hash: "00".into(),
                    players: 1,
                    entities: 1,
                    skill_events_per_second: 1,
                    duration_seconds: 1,
                },
                tick_latency: LatencySummary::from_milliseconds(&[1.0, 2.0, 3.0]).expect("summary"),
                memory_peak_bytes: 1,
                network_bytes_per_second: 1,
                blocking_tick_io_events: 0,
            }
            .write_json(&root.join(format!("{scenario}.json")))
            .expect("write report");
        }
        verify_performance_suite(&root).expect("verify performance suite");
        fs::remove_file(root.join("entity_dense.json")).expect("remove scenario");
        assert!(verify_performance_suite(&root).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }
}

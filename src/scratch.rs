//! Scratch copies of HEAD and planting into them. Nothing here writes to the
//! repository: HEAD is read with `git archive`, plants are only read.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::verdict::PlantState;

pub const DELETE_FILE: &str = "delete";

/// A temp directory removed on drop.
pub struct WorkDir(PathBuf);

impl WorkDir {
    pub fn new() -> io::Result<WorkDir> {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for n in 0..100u32 {
            let p = base.join(format!("plant-{}-{nanos}-{n}", std::process::id()));
            match fs::create_dir(&p) {
                Ok(()) => return Ok(WorkDir(p)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("cannot create a unique temp dir"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = force_remove(&self.0);
    }
}

/// Remove a tree, also when a check left read-only directories in it.
pub fn force_remove(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
        Ok(m) if !m.is_dir() => return fs::remove_file(p),
        Ok(_) => {}
    }
    if fs::remove_dir_all(p).is_ok() {
        return Ok(());
    }
    make_writable(p);
    fs::remove_dir_all(p)
}

fn make_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(m) = fs::symlink_metadata(dir) else {
        return;
    };
    if !m.is_dir() {
        return;
    }
    let mut perms = m.permissions();
    perms.set_mode(perms.mode() | 0o700);
    let _ = fs::set_permissions(dir, perms);
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            make_writable(&e.path());
        }
    }
}

fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("--no-optional-locks").arg("-C").arg(root);
    c
}

fn git_out(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git is not runnable: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Extract `git archive HEAD` of `root` into the new directory `dest`.
pub fn export_head(root: &Path, dest: &Path) -> Result<(), String> {
    git_out(root, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .map_err(|_| format!("{} has no commit to export (HEAD)", root.display()))?;
    fs::create_dir(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut archive = git(root)
        .args(["archive", "--format=tar", "HEAD"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git is not runnable: {e}"))?;
    let tar_in = archive.stdout.take().expect("piped stdout");
    let tar = Command::new("tar")
        .arg("-xf")
        .arg("-")
        .arg("-C")
        .arg(dest)
        .stdin(tar_in)
        .output()
        .map_err(|e| format!("tar is not runnable: {e}"))?;
    let git_res = archive
        .wait_with_output()
        .map_err(|e| format!("git archive: {e}"))?;
    if !git_res.status.success() {
        return Err(format!(
            "git archive failed: {}",
            String::from_utf8_lossy(&git_res.stderr).trim()
        ));
    }
    if !tar.status.success() {
        return Err(format!(
            "tar failed: {}",
            String::from_utf8_lossy(&tar.stderr).trim()
        ));
    }
    Ok(())
}

/// True when tracked files differ from HEAD (those edits are not under test).
pub fn tracked_changes(root: &Path) -> bool {
    git_out(root, &["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Copy a directory tree, keeping symlinks as symlinks.
pub fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&from)?, &to)?;
        } else if ft.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Join `rel` under `base` lexically, refusing absolute paths and anything
/// that climbs out of `base`.
pub fn inside(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => parts.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(format!("path {rel:?} escapes the repo root"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute path {rel:?}"))
            }
        }
    }
    if parts.is_empty() {
        return Err(format!("path {rel:?} names the root itself"));
    }
    let mut p = base.to_path_buf();
    p.extend(parts);
    Ok(p)
}

/// Refuse a target whose parent directories (below `base`) include a symlink,
/// so a write or delete cannot be redirected outside the scratch copy.
fn no_symlinked_parent(base: &Path, target: &Path) -> Result<(), String> {
    let rel = target.strip_prefix(base).map_err(|_| "path outside base")?;
    let mut cur = base.to_path_buf();
    let comps: Vec<_> = rel.components().collect();
    for c in &comps[..comps.len().saturating_sub(1)] {
        cur.push(c);
        if let Ok(m) = fs::symlink_metadata(&cur) {
            if m.file_type().is_symlink() {
                return Err(format!(
                    "{} goes through the symlink {}",
                    rel.display(),
                    cur.strip_prefix(base).unwrap_or(&cur).display()
                ));
            }
        }
    }
    Ok(())
}

/// A planted symlink at `rel` must resolve, lexically, inside the scratch
/// root; otherwise a check that writes through it writes outside.
fn link_stays_inside(rel: &str, target: &Path) -> Result<(), String> {
    let t = target.to_string_lossy();
    if target.is_absolute() {
        return Err(format!("symlink {rel} points to the absolute path {t}"));
    }
    let parent = Path::new(rel).parent().unwrap_or(Path::new(""));
    let joined = parent.join(target);
    inside(Path::new("/"), &joined.to_string_lossy())
        .map(|_| ())
        .map_err(|_| format!("symlink {rel} -> {t} points outside the scratch copy"))
}

/// A plant directory's contents.
pub struct Plant {
    pub dir: PathBuf,
    pub state: PlantState,
    pub files: Vec<String>,
    pub deletes: Vec<String>,
}

/// Read the plant directory `rel` from the working tree under `root`.
/// `Err` means the path itself is unacceptable (escapes the root).
pub fn read_plant(root: &Path, rel: &str) -> Result<Plant, String> {
    let dir = inside(root, rel).map_err(|e| format!("plant: {e}"))?;
    let mut plant = Plant {
        dir: dir.clone(),
        state: PlantState::Missing,
        files: Vec::new(),
        deletes: Vec::new(),
    };
    match fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() => {}
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!(
                "plant dir {rel} is a symlink, which is not allowed"
            ))
        }
        _ => return Ok(plant),
    }
    let (real_dir, real_root) = (fs::canonicalize(&dir), fs::canonicalize(root));
    match (real_dir, real_root) {
        (Ok(d), Ok(r)) if d.starts_with(&r) && d != r => {}
        _ => return Err(format!("plant dir {rel} resolves outside the repo root")),
    }
    walk(&dir, &dir, &mut plant.files).map_err(|e| format!("cannot read plant {rel}: {e}"))?;
    plant.files.retain(|f| f != DELETE_FILE);
    plant.files.sort();
    let dfile = dir.join(DELETE_FILE);
    if fs::symlink_metadata(&dfile)
        .map(|m| m.is_file())
        .unwrap_or(false)
    {
        let text = fs::read_to_string(&dfile)
            .map_err(|e| format!("cannot read {rel}/{DELETE_FILE}: {e}"))?;
        plant.deletes = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
    }
    plant.state = if plant.files.is_empty() && plant.deletes.is_empty() {
        PlantState::Empty
    } else {
        PlantState::Ready
    };
    Ok(plant)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if entry.file_type()?.is_dir() {
            walk(base, &p, out)?;
        } else {
            let rel = p.strip_prefix(base).expect("under base");
            out.push(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

/// Overlay the plant's files onto `scratch`, then apply its delete list.
pub fn apply_plant(scratch: &Path, plant: &Plant) -> Result<(), String> {
    for rel in &plant.files {
        let dst = inside(scratch, rel)?;
        no_symlinked_parent(scratch, &dst)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        }
        if let Ok(m) = fs::symlink_metadata(&dst) {
            // Replace, never write through: a symlink here would redirect the copy.
            if m.is_dir() {
                fs::remove_dir_all(&dst)
            } else {
                fs::remove_file(&dst)
            }
            .map_err(|e| format!("{rel}: {e}"))?;
        }
        let src = plant.dir.join(rel);
        let ft = fs::symlink_metadata(&src)
            .map_err(|e| format!("{rel}: {e}"))?
            .file_type();
        if ft.is_symlink() {
            let target = fs::read_link(&src).map_err(|e| format!("{rel}: {e}"))?;
            link_stays_inside(rel, &target)?;
            std::os::unix::fs::symlink(target, &dst).map_err(|e| format!("{rel}: {e}"))?;
        } else {
            fs::copy(&src, &dst).map_err(|e| format!("{rel}: {e}"))?;
            // fs::copy can keep the source's mtime (macOS does). A planted file
            // older than a build cache is skipped by mtime-based tools like
            // cargo, and the check then runs without the plant: stamp it now.
            fs::File::options()
                .write(true)
                .open(&dst)
                .and_then(|f| f.set_modified(std::time::SystemTime::now()))
                .map_err(|e| format!("{rel}: cannot stamp mtime: {e}"))?;
        }
    }
    for rel in &plant.deletes {
        let target = inside(scratch, rel).map_err(|e| format!("delete: {e}"))?;
        no_symlinked_parent(scratch, &target)?;
        let m = fs::symlink_metadata(&target)
            .map_err(|_| format!("delete lists {rel:?}, which is not in the scratch copy"))?;
        if m.is_dir() {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        }
        .map_err(|e| format!("delete {rel}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // rivet: verifies REQ-004
    #[test]
    fn inside_refuses_escapes() {
        let b = Path::new("/base");
        assert_eq!(inside(b, "a/./b").unwrap(), PathBuf::from("/base/a/b"));
        assert_eq!(inside(b, "a/../c").unwrap(), PathBuf::from("/base/c"));
        assert!(inside(b, "../x").unwrap_err().contains("escapes"));
        assert!(inside(b, "a/../../x").unwrap_err().contains("escapes"));
        assert!(inside(b, "/etc/passwd").unwrap_err().contains("absolute"));
        assert!(inside(b, ".").unwrap_err().contains("root itself"));
    }
}

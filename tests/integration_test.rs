use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::Write;
use std::process::Command as StdCommand;
use tempfile::tempdir;

#[test]
fn test_basic_list() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("file1.txt"))?;
    fs::File::create(dir.path().join("file2.txt"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-1").arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("file1.txt"))
        .stdout(predicate::str::contains("file2.txt"));

    Ok(())
}

#[test]
fn test_hidden_files_filter() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join(".hidden_dot"))?;
    fs::File::create(dir.path().join("visible_file"))?;
    
    let hidden_win = dir.path().join("hidden_win");
    fs::File::create(&hidden_win)?;
    // Set hidden attribute on Windows
    let status = StdCommand::new("attrib").arg("+h").arg(&hidden_win).status()?;
    assert!(status.success());

    // Should NOT show hidden by default
    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains(".hidden_dot").not())
        .stdout(predicate::str::contains("hidden_win").not())
        .stdout(predicate::str::contains("visible_file"));

    // Should show hidden with -a
    let mut cmd_all = Command::cargo_bin("ls_tool")?;
    cmd_all.arg("-a").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains(".hidden_dot"))
        .stdout(predicate::str::contains("hidden_win"))
        .stdout(predicate::str::contains("."))
        .stdout(predicate::str::contains(".."));

    // Should show hidden with -A (almost all)
    let mut cmd_almost = Command::cargo_bin("ls_tool")?;
    cmd_almost.arg("-A").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains(".hidden_dot"))
        .stdout(predicate::str::contains("hidden_win"))
        .stdout(predicate::str::contains(".\n").not())
        .stdout(predicate::str::contains("..\n").not());

    Ok(())
}

#[test]
fn test_sorting_logic() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let f1 = dir.path().join("b.txt");
    let f2 = dir.path().join("a.txt");
    let f3 = dir.path().join("c.txt");
    fs::write(&f1, "small")?;
    fs::write(&f2, "medium length")?;
    fs::write(&f3, "very large content indeed")?;

    // Default alphabetical
    let mut cmd = Command::cargo_bin("ls_tool")?;
    let output = cmd.arg("-1").arg(dir.path()).output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "a.txt");
    assert_eq!(lines[1], "b.txt");
    assert_eq!(lines[2], "c.txt");

    // Reverse sort
    let mut cmd_r = Command::cargo_bin("ls_tool")?;
    let output_r = cmd_r.arg("-1r").arg(dir.path()).output()?;
    let stdout_r = String::from_utf8(output_r.stdout)?;
    let lines_r: Vec<&str> = stdout_r.lines().collect();
    assert_eq!(lines_r[0], "c.txt");
    assert_eq!(lines_r[1], "b.txt");
    assert_eq!(lines_r[2], "a.txt");

    // Size sort
    let mut cmd_s = Command::cargo_bin("ls_tool")?;
    let output_s = cmd_s.arg("-1S").arg(dir.path()).output()?;
    let stdout_s = String::from_utf8(output_s.stdout)?;
    let lines_s: Vec<&str> = stdout_s.lines().collect();
    assert_eq!(lines_s[0], "c.txt"); // largest
    assert_eq!(lines_s[1], "a.txt");
    assert_eq!(lines_s[2], "b.txt"); // smallest

    Ok(())
}

#[test]
fn test_long_format() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test_file.txt");
    fs::write(&file_path, "content")?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-l").arg(dir.path())
        .assert()
        .success()
        // Check for presence of permissions (e.g., -rwx)
        .stdout(predicate::str::is_match(r"[-d][rwx-]{9}")?)
        // Check for owner/group info
        .stdout(predicate::str::contains("test_file.txt"));

    // Test -n (numeric IDs)
    let mut cmd_n = Command::cargo_bin("ls_tool")?;
    cmd_n.arg("-ln").arg(dir.path())
        .assert()
        .success()
        // Should contain SIDs on Windows
        .stdout(predicate::str::is_match(r"S-\d-\d-\d+")?);

    // Test -o (omit group)
    // We'll just check it doesn't crash and has some output
    let mut cmd_o = Command::cargo_bin("ls_tool")?;
    cmd_o.arg("-lo").arg(dir.path())
        .assert()
        .success();

    // Test --author (implies -l)
    let mut cmd_author = Command::cargo_bin("ls_tool")?;
    cmd_author.arg("--author").arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"[-d][rwx-]{9}")?);

    Ok(())
}

#[test]
fn test_classification() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::create_dir(dir.path().join("subdir"))?;
    fs::File::create(dir.path().join("script.bat"))?; // .bat is executable

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-1").arg("-F").arg("--").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("subdir/"))
        .stdout(predicate::str::contains("script.bat*"));

    // --file-type (similar but no * for executables)
    let mut cmd_ft = Command::cargo_bin("ls_tool")?;
    cmd_ft.arg("--file-type").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("subdir/"))
        .stdout(predicate::str::contains("script.bat*").not());

    Ok(())
}

#[test]
fn test_sizes() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_path = dir.path().join("large_file.txt");
    let content = vec![0u8; 2048]; // 2KB
    fs::write(&file_path, content)?;

    // -s prints size in blocks
    let mut cmd_s = Command::cargo_bin("ls_tool")?;
    cmd_s.arg("-s").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::is_match(r"\d+ large_file\.txt")?);

    // -H with -l
    let mut cmd_lh = Command::cargo_bin("ls_tool")?;
    cmd_lh.arg("-lH1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("2.0K"));

    Ok(())
}

#[test]
fn test_recursive() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir)?;
    fs::File::create(subdir.join("inner.txt"))?;
    fs::File::create(dir.path().join("outer.txt"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-R1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("outer.txt"))
        .stdout(predicate::str::contains("subdir:"))
        .stdout(predicate::str::contains("inner.txt"));

    Ok(())
}

#[test]
fn test_directory_listing() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir)?;

    // Without -d, shows contents of subdir
    // With -d, shows subdir itself
    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-d1").arg(&subdir)
        .assert()
        .stdout(predicate::str::contains("subdir"));

    Ok(())
}

#[test]
fn test_inode_and_numeric() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("test.txt"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-i1").arg(dir.path())
        .assert()
        .stdout(predicate::str::is_match(r"\d+ test\.txt")?);

    Ok(())
}

#[test]
fn test_backup_filtering() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("file.txt"))?;
    fs::File::create(dir.path().join("file.txt~"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("file.txt~"));

    let mut cmd_b = Command::cargo_bin("ls_tool")?;
    cmd_b.arg("-B1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("file.txt~").not());

    Ok(())
}

#[test]
fn test_version_sort() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("file1.txt"))?;
    fs::File::create(dir.path().join("file10.txt"))?;
    fs::File::create(dir.path().join("file2.txt"))?;

    // Default: file1, file10, file2 (lexical)
    // Version: file1, file2, file10
    let mut cmd = Command::cargo_bin("ls_tool")?;
    let output = cmd.arg("-1v").arg(dir.path()).output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "file1.txt");
    assert_eq!(lines[1], "file2.txt");
    assert_eq!(lines[2], "file10.txt");

    Ok(())
}

#[test]
fn test_unsorted_all() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join(".hidden"))?;
    fs::File::create(dir.path().join("z.txt"))?;
    fs::File::create(dir.path().join("a.txt"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    // -f implies -a and -U (no sort)
    // Since it's unsorted, we can't be sure of order, but it should contain .hidden
    cmd.arg("-f1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains(".hidden"))
        .stdout(predicate::str::contains("z.txt"))
        .stdout(predicate::str::contains("a.txt"));

    Ok(())
}

#[test]
fn test_quoting_and_escape() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_with_space = dir.path().join("file with space.txt");
    fs::File::create(&file_with_space)?;
    
    // Default (literal)
    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("file with space.txt"));

    // -b / --escape (escapes spaces)
    let mut cmd_b = Command::cargo_bin("ls_tool")?;
    cmd_b.arg("-b").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("file\\ with\\ space.txt"));

    // --quoting-style=shell
    let mut cmd_shell = Command::cargo_bin("ls_tool")?;
    cmd_shell.arg("--quoting-style=shell").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("'file with space.txt'"));

    // --quoting-style=c
    let mut cmd_c = Command::cargo_bin("ls_tool")?;
    cmd_c.arg("--quoting-style=c").arg("-1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("\"file\\ with\\ space.txt\""));

    Ok(())
}

#[test]
fn test_block_size() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_path = dir.path().join("file.txt");
    let content = vec![0u8; 2048]; // 2KB
    fs::write(&file_path, content)?;

    // Default block size (1)
    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-s1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("2048 file.txt"));

    // --block-size=1K
    let mut cmd_k = Command::cargo_bin("ls_tool")?;
    cmd_k.arg("--block-size=1K").arg("-s1").arg(dir.path())
        .assert()
        .stdout(predicate::str::contains("2 file.txt"));

    Ok(())
}

#[test]
fn test_time_sorts() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let f1 = dir.path().join("old.txt");
    let f2 = dir.path().join("new.txt");
    
    fs::File::create(&f1)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    fs::File::create(&f2)?;

    // Sort by modification time (default with -t)
    let mut cmd_t = Command::cargo_bin("ls_tool")?;
    let output = cmd_t.arg("-1t").arg(dir.path()).output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "new.txt");
    assert_eq!(lines[1], "old.txt");

    // -c sort by ctime (creation time on Windows)
    let mut cmd_c = Command::cargo_bin("ls_tool")?;
    let output_c = cmd_c.arg("-1tc").arg(dir.path()).output()?;
    let stdout_c = String::from_utf8(output_c.stdout)?;
    let lines_c: Vec<&str> = stdout_c.lines().collect();
    assert_eq!(lines_c[0], "new.txt");
    assert_eq!(lines_c[1], "old.txt");

    Ok(())
}

#[test]
fn test_layout_flags() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    for i in 1..=5 {
        fs::File::create(dir.path().join(format!("file{}.txt", i)))?;
    }

    // -C (columns) - we'll just check it runs and produces output
    let mut cmd_c = Command::cargo_bin("ls_tool")?;
    cmd_c.arg("-C").arg("-w").arg("40").arg(dir.path())
        .assert()
        .success();

    // -x (across)
    let mut cmd_x = Command::cargo_bin("ls_tool")?;
    cmd_x.arg("-x").arg("-w").arg("40").arg(dir.path())
        .assert()
        .success();

    Ok(())
}

#[test]
fn test_time_style() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("test.txt"))?;

    // --time-style=iso
    let mut cmd_iso = Command::cargo_bin("ls_tool")?;
    cmd_iso.arg("-l").arg("--time-style=iso").arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"\d{4}-\d{2}-\d{2}")?);

    // --time-style=long-iso
    let mut cmd_long = Command::cargo_bin("ls_tool")?;
    cmd_long.arg("-l").arg("--time-style=long-iso").arg(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}")?);

    Ok(())
}

#[test]
fn test_atime() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    fs::File::create(dir.path().join("test.txt"))?;

    let mut cmd = Command::cargo_bin("ls_tool")?;
    cmd.arg("-ltu").arg(dir.path())
        .assert()
        .success();

    Ok(())
}


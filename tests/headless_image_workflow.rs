//! Policy regression for the headless image workflow's PR trust boundary.
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn workflow() -> Value {
    let source = include_str!("../.github/workflows/headless-image.yml");
    serde_saphyr::from_str(source).expect("headless workflow must parse as YAML")
}

fn steps(job: &Value) -> Vec<&Value> {
    job["steps"]
        .as_array()
        .expect("job must contain steps")
        .iter()
        .collect()
}

#[test]
fn wine_preloader_linker_override_is_scoped_to_make() {
    let containerfile = include_str!("../container/headless/Containerfile");
    let make_line = containerfile
        .lines()
        .find(|line| line.starts_with("RUN make -j$(nproc)"))
        .expect("Wine build invocation");
    assert_eq!(
        make_line,
        "RUN make -j$(nproc) wine_preloader_LDFLAGS=-Wl,--no-rosegment"
    );
    assert!(containerfile.contains("ENV LDFLAGS=\"-Wl,-O1,--sort-common,--as-needed\""));
    for line in containerfile
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
    {
        if line.contains("--no-rosegment") {
            assert_eq!(
                line, make_line,
                "linker override must remain preloader-only"
            );
        }
    }
}

#[test]
fn pr_reachable_code_cannot_obtain_package_write() {
    let workflow = workflow();
    assert!(workflow["on"]["pull_request"].is_object());
    let build = &workflow["jobs"]["build-and-test"];
    assert!(build.is_object(), "PR build/test job missing");
    assert_eq!(build["permissions"]["contents"], "read");
    assert_ne!(build["permissions"]["packages"], "write");
    assert!(steps(build)
        .iter()
        .any(|step| step["uses"] == "actions/checkout@v4"
            && step["with"]["persist-credentials"] == false));
    assert!(steps(build).iter().any(|step| step["run"]
        .as_str()
        .is_some_and(|run| run.contains("scripts/test-headless-entrypoint.py"))));

    for (name, job) in workflow["jobs"].as_object().expect("jobs mapping") {
        if job["permissions"]["packages"] == "write" {
            let condition = job["if"].as_str().unwrap_or_default();
            assert!(
                condition.contains("github.event_name != 'pull_request'"),
                "package-write job {name} must exclude PRs"
            );
            assert!(
                steps(job)
                    .iter()
                    .all(|step| step["uses"] != "actions/checkout@v4"),
                "package-write job {name} must not checkout PR-controlled source"
            );
        }
    }
}

#[test]
fn publisher_requires_successful_native_gate_and_independent_image_pins() {
    let workflow = workflow();
    let build = &workflow["jobs"]["build-and-test"];
    let publish = &workflow["jobs"]["publish"];
    assert!(
        publish.is_object(),
        "separate package-write publisher missing"
    );
    assert_eq!(publish["needs"], "build-and-test");
    assert_eq!(publish["permissions"]["packages"], "write");
    assert_eq!(
        build["outputs"]["image_id"],
        "${{ steps.build.outputs.image_id }}"
    );
    assert_eq!(
        build["outputs"]["archive_sha256"],
        "${{ steps.archive.outputs.sha256 }}"
    );
    let build_script = steps(build)
        .iter()
        .filter_map(|step| step["run"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(build_script.contains("scripts/test-headless-entrypoint.py"));
    assert!(build_script.contains("podman save --format oci-archive"));
    let publisher_script = steps(publish)
        .iter()
        .filter_map(|step| step["run"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        publisher_script.contains("sha256sum -c"),
        "must check cross-job archive pin"
    );
    assert!(
        publisher_script.contains("podman load"),
        "must load the tested artifact"
    );
    assert!(
        !publisher_script.contains("podman build"),
        "must not rebuild for publication"
    );
    let verify = steps(publish)
        .into_iter()
        .find(|step| step["name"] == "Verify transferred image")
        .expect("publisher must verify before login");
    assert!(verify["env"]["EXPECTED_SHA256"]
        .as_str()
        .unwrap_or_default()
        .contains("needs.build-and-test.outputs.archive_sha256"));
    assert!(verify["env"]["EXPECTED_IMAGE_ID"]
        .as_str()
        .unwrap_or_default()
        .contains("needs.build-and-test.outputs.image_id"));
    let verify_script = verify["run"].as_str().expect("verification script");
    assert!(verify_script.contains("sha256sum -c"));
    assert!(verify_script.contains("podman image inspect"));
    assert!(!verify_script.contains("podman login") && !verify_script.contains("podman push"));
    let publisher_steps = steps(publish);
    let verify_position = publisher_steps
        .iter()
        .position(|step| step["name"] == "Verify transferred image")
        .expect("verification position");
    let push_position = publisher_steps
        .iter()
        .position(|step| {
            step["run"]
                .as_str()
                .is_some_and(|run| run.contains("podman push"))
        })
        .expect("publish step");
    assert!(
        verify_position < push_position,
        "verification must precede credentials"
    );
    assert!(publisher_steps[..=verify_position]
        .iter()
        .all(|step| step["run"]
            .as_str()
            .is_none_or(|run| !run.contains("podman login") && !run.contains("podman tag"))));
}

#[test]
fn publisher_refuses_archive_and_identity_mismatch_before_credentials() {
    let workflow = workflow();
    let verify = steps(&workflow["jobs"]["publish"])
        .into_iter()
        .find(|step| step["name"] == "Verify transferred image")
        .expect("verification step");
    let script = verify["run"].as_str().expect("verification shell");
    let dir = tempfile::tempdir().expect("fixture directory");
    let archive = dir.path().join("image.oci.tar");
    fs::write(&archive, b"test-only archive bytes").expect("fixture archive");
    let actual = Command::new("sha256sum")
        .arg(&archive)
        .output()
        .expect("system sha256sum");
    assert!(actual.status.success());
    let checksum = String::from_utf8(actual.stdout).expect("checksum output");
    let checksum = checksum.split_whitespace().next().expect("checksum digest");

    // Test-only command capture: never load or publish a real image here.
    let mock = dir.path().join("podman");
    fs::write(
        &mock,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CALLS\"\ncase \"$1 $2\" in\n  'load --input') exit 0 ;;\n  'image inspect') printf '%s\\n' 2222222222222222222222222222222222222222222222222222222222222222; exit 0 ;;\nesac\nexit 99\n",
    )
    .expect("fake Podman fixture");
    fs::set_permissions(&mock, fs::Permissions::from_mode(0o700)).expect("executable fixture");
    let calls = dir.path().join("podman-calls");
    let github_env = dir.path().join("github-env");
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap()
    );

    let execute = |sha: &str| {
        Command::new("bash")
            .arg("-c")
            .arg(script)
            .env("PATH", &path)
            .env("ARCHIVE", &archive)
            .env("EXPECTED_SHA256", sha)
            .env(
                "EXPECTED_IMAGE_ID",
                "1111111111111111111111111111111111111111111111111111111111111111",
            )
            .env("GITHUB_ENV", &github_env)
            .env("CALLS", &calls)
            .output()
            .expect("verification shell runs")
    };
    let bad_archive = execute(&"0".repeat(64));
    assert!(
        !bad_archive.status.success(),
        "bad archive digest was accepted"
    );
    assert!(!calls.exists(), "archive mismatch reached Podman");
    assert!(
        !github_env.exists(),
        "archive mismatch exported an image ID"
    );

    let bad_identity = execute(checksum);
    assert!(
        !bad_identity.status.success(),
        "wrong image ID was accepted"
    );
    let calls = fs::read_to_string(calls).expect("mock Podman calls");
    assert!(calls.contains("load --input"));
    assert!(calls.contains("image inspect"));
    assert!(!calls.contains("login") && !calls.contains("tag") && !calls.contains("push"));
    assert!(
        !github_env.exists(),
        "identity mismatch exported an image ID"
    );
}

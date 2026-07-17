fn main() {
    let version = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("Hackathon package version must be valid SemVer");
    let mut identifiers = version.pre.as_str().split('.');
    let is_hackathon = matches!(identifiers.next(), Some("hackathon"))
        && identifiers.next().is_some_and(|revision| {
            !revision.is_empty() && revision.chars().all(|character| character.is_ascii_digit())
        })
        && identifiers.next().is_none();

    assert!(
        is_hackathon,
        "This branch builds only numbered Hackathon editions (*-hackathon.N)"
    );

    tauri_build::build()
}

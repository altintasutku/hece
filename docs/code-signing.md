# Code signing

Releases are not signed yet (#49), so Windows SmartScreen warns on first run. The plan is the
free [SignPath Foundation](https://signpath.org/) program for open-source projects; the
application draft is in issue #49. The release workflow already contains the signing steps and
skips them until SignPath is configured.

## What the workflow does

`.github/workflows/release.yml`, after `scripts/build-release.ps1`:

1. Uploads `target/dist/Hece_<version>_x64-setup.exe` as the artifact `hece-windows-unsigned`.
2. Submits it to SignPath with `signpath/github-action-submit-signing-request@v3`, waits, and
   downloads the signed file to `target/signed/`.
3. Replaces the installer in `target/dist/` and rewrites `SHA256SUMS.txt`.

Tag builds use the signing policy `release-signing`; manual test runs use `test-signing`. The
build attestation and the draft release then cover the signed installer.

All steps run only when the repository variable `SIGNPATH_ORGANIZATION_ID` is set.

## Turning it on (after SignPath approves the project)

1. In SignPath: create the project (slug, for example `hece`), an artifact configuration for a
   single PE file (the NSIS installer), and the signing policies `test-signing` and
   `release-signing`. Link the project to this repository's GitHub Actions (trusted build system).
2. In GitHub → Settings → Secrets and variables → Actions:
   - Variables: `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`,
     `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`.
   - Secret: `SIGNPATH_API_TOKEN` (a SignPath CI user token).
3. Run the Release workflow by hand once and check the signature of the artifact
   (`Get-AuthenticodeSignature Hece_*_x64-setup.exe`).
4. Remove the unsigned-build notes, which exist only because releases are unsigned:
   - `README.md`: the SmartScreen block in the install section.
   - `docs/releases/<tag>.md`: the "Unsigned build" note (new releases).
   - `site/index.html`: the SmartScreen FAQ entry.
   Search for "SmartScreen" to find them all.

## Later

The installer is signed; the executables inside it (`hecew.exe`, `hece.exe`,
`hece-engine.exe`, `hece-settings.exe`) are not. Files the installer writes carry no
mark-of-the-web, so SmartScreen does not check them, but signing them too (a second SignPath
request on the binaries before the NSIS step) would make antivirus heuristics happier. Do that
once the first signed release works.

# Local build and delivery

- Keep one current runnable build at `release\tomas-commander.exe`.
  Build and publish it with `.\scripts\build.ps1`; never publish test features.
- `target` is ignored Cargo cache, not a user launch location. Do not create
  per-task target directories such as `ux-release` or `answer-release`.
- For temporary test-feature builds, use
  `.\scripts\build.ps1 -Mode Diagnostic -Features native-desktop-tests`
  (or the specific required test feature). They stay in `target\diagnostic`.
  Close their processes, then run
  `.\scripts\build.ps1 -Mode CleanDiagnostic` after testing.
- At the end of implementation work, leave a verified current executable in
  `release` and give its path in the final response. A failed build must not
  replace the last working executable. Do not call a cached old build current
  when the changed source has not built successfully.
- Preserve historical test identities, but label obsolete binary paths as
  historical. Do not retain old executable copies for those records.
- Published versions use semantic tags and GitHub Releases; v0.1.0 is the first
  experimental Windows x64 release. Do not create tags, releases, commits or
  pushes without an explicit owner request. Publish only the default executable
  built by `scripts\build.ps1`, never diagnostic-feature binaries.

## 2026-11-25 - Stop the console resume test's serve daemon with its helpers
<!-- devlog-id: 2026-11-25-issue-4041-console-daemon-group -->

### Result

- Run the serve daemon in the console resume test in its own process group, and
  on drop stop the whole group and wait until it is empty before the fixture
  directory is removed.

### Why / context

- Killing only the daemon left an in-flight fake tmux call free to recreate
  tmux.log while the tempdir was being removed, and the removal then failed
  silently.

### Evidence

- On main, the unmodified tempdir probe left a lone tmux.log in about one of
  every seven repeats of the test; with the fix it left nothing in 60 repeats.

### Links

- <https://github.com/sympoies/nils-cli/issues/4041>

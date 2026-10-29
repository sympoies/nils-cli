## 2026-10-29 - Allow project read-back launch configuration
<!-- devlog-id: 2026-10-29-project-readback-launch-env -->

### Result

- The default launch policy admits the project read-back switch.

### Why / context

- The runtime hook owns interpretation of the supplied paths; the session
  boundary carries the value unchanged.

### Evidence

- The launch environment fixture reproduces refusal before the default policy
  change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3086>

## 2026-11-20 - Native notice authorization reads no longer fail on macOS when the request arrives late
<!-- devlog-id: 2026-11-20-issue-3850-native-notice-accept-blocking -->

### Result

- The accepted native notice socket is set blocking, so a request that arrives
  after the connection no longer reports authorization-timeout instead of the
  composer refusal.

### Why / context

- macOS accept inherits the nonblocking listener flag, so an early read
  returned WouldBlock, which the refusal mapping reports as
  authorization-timeout; Linux does not inherit it.

### Evidence

- A new test sends the request 500 ms after connecting; the repeated
  instrumented macOS PR job covers the composer and refusal tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3850>

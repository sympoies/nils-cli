# Research CLI documentation

- [Request/result/error contract and acceptance](specs/contract.md)
- [JSON Schema](specs/schema.json)
- [Completion contract](specs/completion.md)

Run `cargo test -p nils-research-cli` for offline native-adapter fixtures,
public-fetch synthetic HTTP tests, CLI process exit checks and HTTP engine
parity. Provider fixtures are synthetic examples of the documented native
Responses/Messages structures. They are not records of paid API requests.

Optional live smoke commands and presence-only skips are documented in the
contract. Missing keys do not prevent plain public URL reads or offline tests.
An unavailable requested model/tool is an error, with no substitution.

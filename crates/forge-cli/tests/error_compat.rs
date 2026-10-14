use forge_cli::error::ForgeError;

#[test]
fn validation_variant_retains_its_published_construction_shape() {
    let error = ForgeError::Validation {
        schema_version: "forge-cli.identity.v1".to_owned(),
        kind: "validation_failed",
        message: "validation failed".to_owned(),
        detail: None,
    };

    assert_eq!(error.exit_code(), nils_common::cli_contract::exit::DATA);
}

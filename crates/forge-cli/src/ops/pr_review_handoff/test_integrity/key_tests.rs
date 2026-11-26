//! Per-test row keys: module paths, retired/stale rows and JavaScript titles.
use super::*;
use pretty_assertions::assert_eq;

fn deleted(path: &str, before: &str, after: &str) -> Vec<String> {
    kinds(&patch(path, before, after))
}

fn detail(error: ForgeError) -> String {
    let ForgeError::Validation {
        detail: Some(detail),
        ..
    } = error
    else {
        panic!("expected validation details")
    };
    detail
}

const TWO_MODULES: &str =
    "mod first {\n    #[test]\n    fn same() {}\n}\nmod second {\n    #[test]\n    fn same() {}\n}";
const FIRST_ONLY: &str = "mod first {\n    #[test]\n    fn same() {}\n}";
const SECOND_ONLY: &str = "mod second {\n    #[test]\n    fn same() {}\n}";

#[test]
fn same_named_rust_tests_in_different_modules_get_separate_rows() {
    assert_eq!(
        deleted("src/lib.rs", TWO_MODULES, FIRST_ONLY),
        vec!["deleted-test:second::same"]
    );
    assert_eq!(
        deleted("src/lib.rs", TWO_MODULES, SECOND_ONLY),
        vec!["deleted-test:first::same"]
    );
    let nested = "mod outer {\n    mod inner {\n        #[test]\n        fn same() {}\n    }\n    #[test]\n    fn same() {}\n}";
    let kept = "mod outer {\n    mod inner {\n        #[test]\n        fn same() {}\n    }\n}";
    assert_eq!(
        deleted("src/lib.rs", nested, kept),
        vec!["deleted-test:outer::same"]
    );
}

#[test]
fn a_row_for_one_module_does_not_cover_the_same_name_in_another() {
    let patch = patch("src/lib.rs", TWO_MODULES, FIRST_ONLY);
    for name in ["same", "first::same"] {
        let body = format!(
            "## Test changes\n- src/lib.rs | deleted-test:{name} | Another test keeps this contract."
        );
        let metadata = subject(HEAD, BASE, &body);
        assert_eq!(
            admit(
                metadata.clone(),
                metadata,
                &patch,
                &confirmation(&patch, BASE, HEAD, &body)
            )
            .unwrap_err()
            .kind(),
            "test_changes_undeclared"
        );
    }
}

#[test]
fn the_retired_file_level_row_and_stale_rows_are_reported() {
    let patch = patch(
        "src/lib.rs",
        "#[test]\nfn keeps() {}\n#[test]\nfn drops() {}",
        "#[test]\nfn keeps() {}",
    );
    let body = "## Test changes\n- src/lib.rs | deleted-test | Another test keeps this contract.\n- src/lib.rs | deleted-test:renamed_away | Another test keeps this contract.";
    let metadata = subject(HEAD, BASE, body);
    let error = admit(
        metadata.clone(),
        metadata,
        &patch,
        &confirmation(&patch, BASE, HEAD, body),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "test_changes_undeclared");
    let detail = detail(error);
    assert!(
        detail.contains("Retired file-level form")
            && detail.contains("- src/lib.rs | deleted-test\\n"),
        "{detail}"
    );
    assert!(
        detail.contains("match no finding") && detail.contains("deleted-test:renamed_away"),
        "{detail}"
    );
    assert!(detail.contains("deleted-test:drops"), "{detail}");
}

#[test]
fn javascript_titles_on_their_own_line_are_extracted() {
    let before = "it(\n  'handles x',\n  () => {},\n);\ntest(\n\n  // title follows\n  \"handles y\", () => {});\nit('keeps', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('keeps', () => {});"),
        vec![
            "deleted-test:it('handlesx'",
            "deleted-test:test(\"handlesy\""
        ]
    );
}

#[test]
fn javascript_titles_with_escaped_quotes_are_extracted() {
    let before = "it('it\\'s a', () => {});\nit('it\\'s b', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('it\\'s a', () => {});"),
        vec!["deleted-test:it('it\\'sb'"]
    );
}

#[test]
fn javascript_each_tables_are_one_row_named_by_the_table_title() {
    let before = "it.each([[1, 2]])('adds %i', () => {});\ntest.each([\n  [1, 2],\n  [3, 4],\n])(\n  'sums %i',\n  () => {},\n);\nit.each`\n  a | b\n  ${1} | ${2}\n`('tags $a', () => {});\nit('keeps', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('keeps', () => {});"),
        vec![
            "deleted-test:it.each('adds%i'",
            "deleted-test:it.each('tags$a'",
            "deleted-test:test.each('sums%i'"
        ]
    );
}

#[test]
fn javascript_titles_with_path_separators_keep_distinct_labels() {
    let before =
        "it('a::function x', () => {});\nit('a::fn tionx', () => {});\nit('keeps', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('keeps', () => {});"),
        vec![
            "deleted-test:it('a::fntionx'",
            "deleted-test:it('a::functionx'"
        ]
    );
}

#[test]
fn module_nesting_does_not_continue_across_hunks() {
    // The first hunk ends inside `mod first {`; the second removes a top-level test.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n mod first {\n-    const A: u8 = 1;\n+    const A: u8 = 2;\n@@ -50,2 +50,0 @@\n-#[test]\n-fn same() {}\n";
    assert_eq!(kinds(patch), vec!["deleted-test:same"]);
}

#[test]
fn stale_and_retired_rows_warn_even_when_every_finding_is_covered() {
    let patch = patch(
        "src/lib.rs",
        "#[test]\nfn keeps() {}\n#[test]\nfn drops() {}",
        "#[test]\nfn keeps() {}",
    );
    let body = "## Test changes\n- src/lib.rs | deleted-test:drops | Another test keeps this contract.\n- src/lib.rs | deleted-test | Another test keeps this contract.\n- src/lib.rs | assertion-change | Another test keeps this contract.";
    let metadata = subject(HEAD, BASE, body);
    admit(
        metadata.clone(),
        metadata,
        &patch,
        &confirmation(&patch, BASE, HEAD, body),
    )
    .unwrap();
    let findings = diff::detect(&patch).unwrap();
    let warnings = row_warnings(&findings, &declared_rows(&declarations(body)));
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].starts_with("Retired file-level form"),
        "{warnings:?}"
    );
    assert!(
        warnings[0].ends_with("- src/lib.rs | deleted-test"),
        "{warnings:?}"
    );
    assert!(warnings[1].contains("match no finding"), "{warnings:?}");
    assert!(
        warnings[1].ends_with("- src/lib.rs | assertion-change"),
        "{warnings:?}"
    );
}

#[test]
fn python_decorator_changes_preserve_test_identity() {
    let plain = "    def test_contract(self):\n        assert True";
    let decorated = "    @mock.patch('example.call')\n    def test_contract(self, mocked_call):\n        assert True";
    for (before, after) in [(plain, decorated), (decorated, plain)] {
        let patch = patch("tests/test_api.py", before, after);
        assert_eq!(kinds(&patch), Vec::<String>::new());
        let metadata = subject(HEAD, BASE, "");
        admit(metadata.clone(), metadata, &patch, "").unwrap();
    }
    let same_signature =
        "    @mock.patch('example.call')\n    def test_contract(self):\n        assert True";
    for (before, after) in [(plain, same_signature), (same_signature, plain)] {
        assert_eq!(
            deleted("tests/test_api.py", before, after),
            Vec::<String>::new()
        );
    }
    assert_eq!(
        deleted(
            "tests/test_api.py",
            plain,
            "    def test_renamed(self):\n        assert True"
        ),
        vec!["deleted-test:test_contract"]
    );
}

#[test]
fn rust_attributes_preserve_identity_and_javascript_wrappers_change_scope() {
    let rust = "#[test]\nfn contract() {}";
    let attributed = "#[allow(unused)]\n#[test]\nfn contract() {}";
    for (before, after) in [(rust, attributed), (attributed, rust)] {
        assert_eq!(deleted("src/lib.rs", before, after), Vec::<String>::new());
    }
    let js = "test('contract', () => {});";
    let wrapped = "describe('group', () => {\n  test('contract', () => {});\n});";
    assert_eq!(
        deleted("api.test.ts", js, wrapped),
        ["deleted-test:test('contract'"]
    );
    assert_eq!(
        deleted("api.test.ts", wrapped, js),
        ["deleted-test:describe('group'::test('contract'"]
    );
}

#[test]
fn javascript_non_it_titles_with_path_separators_keep_distinct_labels() {
    let before = "test.only('a::function x', () => {});\ntest.only('a::fn tionx', () => {});";
    assert_eq!(
        deleted("api.test.ts", before, ""),
        vec![
            "deleted-test:test.only('a::fntionx'",
            "deleted-test:test.only('a::functionx'"
        ]
    );
}

#[test]
fn test_markers_only_count_as_code_in_their_own_language() {
    for (path, before, after) in [
        (
            "src/lib.rs",
            "let marker = \"#[cfg(test)]\";\nif rust_test || line.starts_with(\"@Test\") {\n    work();\n}",
            "if rust_test {\n    work();\n}",
        ),
        (
            "src/lib.rs",
            "const SAMPLE: &str = r#\"\n#[test]\nfn example() {}\n\"#;",
            "",
        ),
        (
            "api.py",
            "example = \"\"\"\ndef test_example():\n    pass\n\"\"\"",
            "",
        ),
        (
            "Api.java",
            "String example = \"\"\"\n@Test\nvoid example() {}\n\"\"\";",
            "",
        ),
        (
            "api.ts",
            "const example = `\ntest('example', () => {});\n`;",
            "",
        ),
        (
            "api.rs",
            "",
            "let examples = \"#[ignore] t.Skip() @Disabled pytest.mark.skip\";",
        ),
    ] {
        assert_eq!(deleted(path, before, after), Vec::<String>::new(), "{path}");
    }
}

#[test]
fn multi_hunk_deletions_use_both_immutable_images() {
    for (path, first, second, close, declaration, expected) in [
        (
            "src/lib.rs",
            "mod first {",
            "mod second {",
            "}",
            "#[test]\nfn same() {}",
            "first::same",
        ),
        (
            "src/lib.rs",
            "impl First {",
            "impl Second {",
            "}",
            "#[rstest]\nfn same() {}",
            "First::same",
        ),
        (
            "api.test.ts",
            "describe('first', () => {",
            "describe('second', () => {",
            "});",
            "test('same::name', () => {});",
            "describe('first'::test('same::name'",
        ),
        (
            "api.py",
            "class First(\n    Parent,\n):",
            "class Second:",
            "",
            "def test_same(self): pass",
            "First::test_same",
        ),
        (
            "Api.java",
            "class First {",
            "class Second {",
            "}",
            "@Test\nvoid same() {}",
            "First::same",
        ),
    ] {
        let filler = if path.ends_with(".py") {
            "    pass\n"
        } else {
            "    // unchanged\n"
        };
        let gap = filler.repeat(8);
        let first = format!("{first}\n{gap}");
        let middle = format!("{gap}{close}\n{second}\n{gap}");
        let last = format!("{gap}{close}\n");
        let declaration = format!("    {}", declaration.replace('\n', "\n    "));
        let before = format!("{first}{declaration}\n{middle}{last}");
        let after = format!("{first}{middle}{declaration}\n{last}");
        let length = declaration.lines().count();
        let removed = declaration.replace('\n', "\n-");
        let added = declaration.replace('\n', "\n+");
        let second = first.lines().count() + length + middle.lines().count();
        let patch = format!(
            "diff --git a/{path} b/{path}\n@@ -{},{length} +{},0 @@\n-{removed}\n@@ -{second},0 +{},{length} @@\n+{added}\n",
            first.lines().count() + 1,
            first.lines().count(),
            second - length + 1
        );
        let mut runner =
            ComparisonRunner::new(&patch, [subject(HEAD, BASE, ""), subject(HEAD, BASE, "")]);
        runner.images.add(&mut runner.files[0], &before, &after);
        let (_, _, findings) = comparison(&runner, &context(), 7, HEAD).unwrap();
        assert_eq!(
            findings,
            [diff::Finding {
                path: path.into(),
                kind: format!("deleted-test:{expected}"),
            }],
            "{path}"
        );
    }
}

#[test]
fn javascript_nested_suites_resolve_multiline_and_inline_openers() {
    let before = "describe(\n  'outer braces }',\n  () => {\n    describe.only('inner', () => { test('case', () => {}); });\n  },\n);";
    let after = before.replace("test('case', () => {});", "");
    assert_eq!(
        deleted("api.test.ts", before, &after),
        ["deleted-test:describe('outer%{20}braces%{20}}'::describe('inner'::test('case'"]
    );
}

#[test]
fn recognizable_test_declarations_inside_rust_macros_keep_the_module_path() {
    let before = "mod outer {\n    macro_rules! tests {\n        () => {\n            #[test] fn concrete() {}\n        };\n    }\n    tests!();\n}";
    let after = before.replace("#[test] fn concrete() {}", "");
    assert_eq!(
        deleted("src/lib.rs", before, &after),
        ["deleted-test:outer::concrete"]
    );
}

#[test]
fn javascript_regex_literals_preserve_later_test_deletions() {
    let inputs = [
        "const pattern = /`/g;",
        r"const pattern = /src\/*.ts/;",
        "const pattern = /[/`]/;",
        r#"const ratio = "12" / /* } */ 2;"#,
    ];
    let findings = inputs.map(|input| {
        let suite = format!("describe('suite', () => {{\n    {input}\n");
        deleted(
            "api.test.ts",
            &format!("{suite}    test('removed', () => {{}});\n}});"),
            &format!("{suite}}});"),
        )
    });
    assert_eq!(
        findings,
        inputs.map(|_| vec!["deleted-test:describe('suite'::test('removed'".to_string()])
    );
}

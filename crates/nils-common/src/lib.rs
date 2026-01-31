pub fn greeting(name: &str) -> String {
    format!("Hello, {name}!")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn greeting_formats_name() {
        let result = greeting("Nils");
        assert_eq!(result, "Hello, Nils!");
    }
}

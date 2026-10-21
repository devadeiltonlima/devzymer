const NATIVE_PHRASES: [&str; 28] = [
    "use grep",
    "usar grep",
    "use o grep",
    "usa o grep",
    "usando grep",
    "with grep",
    "com grep",
    "via grep",
    "use ripgrep",
    "use rg ",
    "com ripgrep",
    "use findstr",
    "use select-string",
    "native search",
    "busca nativa",
    "pesquisa nativa",
    "grep instead",
    "rg instead",
    "use devzymer not",
    "without devzymer",
    "without using devzymer",
    "sem devzymer",
    "sem usar o devzymer",
    "ignore devzymer",
    "ignore o devzymer",
    "skip devzymer",
    "disable devzymer",
    "desative o devzymer",
];

const NEGATIONS: [&str; 9] = [
    "n't", "not ", "nao ", "não ", "never", "evite", "avoid", "nunca", "no ",
];

pub fn requests_native_search(prompt: &str) -> bool {
    let normalized: String = prompt
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let padded = format!("{normalized} ");
    NATIVE_PHRASES.iter().any(|phrase| {
        let mut from = 0;
        while let Some(found) = padded[from..].find(phrase) {
            let start = from + found;
            let mut before_start = start.saturating_sub(16);
            while before_start > 0 && !padded.is_char_boundary(before_start) {
                before_start -= 1;
            }
            let before = &padded[before_start..start];
            let negated = NEGATIONS.iter().any(|negation| before.contains(negation))
                && !phrase.contains("without")
                && !phrase.contains("sem ")
                && !phrase.contains("ignore")
                && !phrase.contains("skip")
                && !phrase.contains("disable")
                && !phrase.contains("desative");
            if !negated {
                return true;
            }
            from = start + phrase.len();
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_request_for_native_search_is_recognized() {
        for prompt in [
            "Please use grep to find the handler",
            "use ripgrep on the src folder",
            "Faça a busca com grep, por favor",
            "use o grep para achar isso",
            "do a native search for the config",
            "sem devzymer dessa vez",
            "ignore devzymer and search the repo",
            "run it without using devzymer",
            "USE   GREP\nnow",
        ] {
            assert!(requests_native_search(prompt), "{prompt}");
        }
    }

    #[test]
    fn a_negated_mention_is_not_a_request() {
        for prompt in [
            "don't use grep here, ask devzymer",
            "do not use grep",
            "não use o grep",
            "nunca use grep neste projeto",
            "avoid use grep",
        ] {
            assert!(!requests_native_search(prompt), "{prompt}");
        }
    }

    #[test]
    fn ordinary_prompts_are_not_requests() {
        for prompt in [
            "Fix the failing test in the billing module",
            "Where is useVoiceCall defined?",
            "explain how grep works",
            "",
        ] {
            assert!(!requests_native_search(prompt), "{prompt}");
        }
    }
}

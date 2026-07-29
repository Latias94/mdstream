use pulldown_cmark::{BrokenLink, CowStr, Options};

pub(super) fn canonical_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
}

pub(super) fn preserve_broken_reference(_link: BrokenLink<'_>) -> Option<(CowStr<'_>, CowStr<'_>)> {
    Some((CowStr::from(""), CowStr::from("")))
}

#[cfg(test)]
mod tests {
    use pulldown_cmark::Options;

    use super::canonical_options;

    #[test]
    fn profile_enables_only_the_pinned_extensions() {
        let options = canonical_options();

        assert!(options.contains(Options::ENABLE_TABLES));
        assert!(options.contains(Options::ENABLE_FOOTNOTES));
        assert!(options.contains(Options::ENABLE_STRIKETHROUGH));
        assert!(options.contains(Options::ENABLE_TASKLISTS));
        assert!(options.contains(Options::ENABLE_MATH));
        assert!(options.contains(Options::ENABLE_GFM));
        assert!(!options.contains(Options::ENABLE_HEADING_ATTRIBUTES));
        assert!(!options.contains(Options::ENABLE_WIKILINKS));
        assert!(!options.contains(Options::ENABLE_SUPERSCRIPT));
        assert!(!options.contains(Options::ENABLE_SUBSCRIPT));
        assert!(!options.contains(Options::ENABLE_DEFINITION_LIST));
        assert!(!options.contains(Options::ENABLE_SMART_PUNCTUATION));
    }
}

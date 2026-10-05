//! Snippet keywords typed in other apps: the characters typed since the cursor last jumped, matched
//! against the keywords of `text` shortcuts that have `"expand": true`.

const MAX_TYPED: usize = 64;

#[derive(Default)]
pub struct Typed {
    buf: String,
    keywords: Vec<String>,
}

impl Typed {
    pub fn set_keywords(&mut self, keywords: Vec<String>) {
        self.keywords = keywords.into_iter().filter(|k| !k.is_empty()).collect();
        self.keywords.sort_by_key(|k| std::cmp::Reverse(k.chars().count()));
        self.buf.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }

    /// The cursor moved, the app changed or text was pasted: what came before is unknown.
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    pub fn backspace(&mut self) {
        self.buf.pop();
    }

    /// Add typed characters. Returns the keyword that now ends the typed text (the longest, when
    /// one keyword ends another) and starts over.
    pub fn push(&mut self, chars: &str) -> Option<String> {
        for c in chars.chars() {
            if c.is_control() {
                self.buf.clear();
            } else {
                self.buf.push(c);
            }
        }
        let extra = self.buf.chars().count().saturating_sub(MAX_TYPED);
        if extra > 0 {
            let cut = self.buf.char_indices().nth(extra).map_or(self.buf.len(), |(i, _)| i);
            self.buf.drain(..cut);
        }
        let hit = self.keywords.iter().find(|k| self.buf.ends_with(k.as_str())).cloned();
        if hit.is_some() {
            self.buf.clear();
        }
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keywords: &[&str]) -> Typed {
        let mut t = Typed::default();
        t.set_keywords(keywords.iter().map(|k| k.to_string()).collect());
        t
    }

    fn type_all(t: &mut Typed, s: &str) -> Vec<String> {
        s.chars().filter_map(|c| t.push(&c.to_string())).collect()
    }

    #[test]
    fn matches_keywords_at_the_end_of_what_was_typed() {
        let mut t = typed(&[";sig", "sig", ";addr"]);
        assert_eq!(type_all(&mut t, "hello ;sig"), [";sig"], "the longer keyword wins");
        assert_eq!(type_all(&mut t, "sig"), ["sig"], "starts over after a match");
        assert_eq!(type_all(&mut t, ";adx"), Vec::<String>::new());
        t.backspace();
        assert_eq!(type_all(&mut t, "dr"), [";addr"]);
        assert_eq!(type_all(&mut t, ";SIG"), Vec::<String>::new(), "keywords are case-sensitive");
    }

    #[test]
    fn reset_and_control_characters_forget_what_was_typed() {
        let mut t = typed(&[";sig"]);
        type_all(&mut t, ";si");
        t.reset();
        assert!(type_all(&mut t, "g").is_empty());
        type_all(&mut t, ";si\r");
        assert!(type_all(&mut t, "g").is_empty());
        assert_eq!(type_all(&mut t, "é;sig"), [";sig"]);
    }

    #[test]
    fn keeps_only_recent_characters() {
        let mut t = typed(&[";sig"]);
        type_all(&mut t, &"é".repeat(500));
        assert_eq!(t.buf.chars().count(), MAX_TYPED);
        assert_eq!(type_all(&mut t, ";sig"), [";sig"]);
        assert!(typed(&[]).is_empty());
    }
}

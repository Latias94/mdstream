#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct NewlineNormalizer {
    pending_cr: bool,
}

impl NewlineNormalizer {
    pub(super) fn pending_bytes(self) -> usize {
        usize::from(self.pending_cr)
    }

    pub(super) fn append(self, chunk: &str) -> (Self, String) {
        if chunk.is_empty() {
            return (self, String::new());
        }

        let mut next = self;
        let mut output = String::with_capacity(chunk.len() + usize::from(self.pending_cr));
        let mut characters = chunk.chars().peekable();

        if next.pending_cr {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            output.push('\n');
            next.pending_cr = false;
        }

        while let Some(character) = characters.next() {
            if character != '\r' {
                output.push(character);
                continue;
            }
            if characters.peek() == Some(&'\n') {
                characters.next();
                output.push('\n');
            } else if characters.peek().is_none() {
                next.pending_cr = true;
            } else {
                output.push('\n');
            }
        }

        (next, output)
    }

    pub(super) fn finish(self) -> (Self, String) {
        if self.pending_cr {
            (Self::default(), "\n".to_string())
        } else {
            (self, String::new())
        }
    }
}

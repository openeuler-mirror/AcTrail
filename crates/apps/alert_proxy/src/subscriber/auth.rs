use subtle::ConstantTimeEq;

pub(super) struct TokenVerifier {
    tokens: Box<[Box<[u8]>]>,
}

impl TokenVerifier {
    pub(super) fn new(tokens: &[String]) -> Self {
        Self {
            tokens: tokens
                .iter()
                .map(|token| token.as_bytes().to_vec().into_boxed_slice())
                .collect(),
        }
    }

    pub(super) fn accepts(&self, candidate: &str) -> bool {
        let candidate = candidate.as_bytes();
        let mut accepted = 0_u8;
        for token in &self.tokens {
            if token.len() == candidate.len() {
                accepted |= token.as_ref().ct_eq(candidate).unwrap_u8();
            }
        }
        accepted == 1
    }
}

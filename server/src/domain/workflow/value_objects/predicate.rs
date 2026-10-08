#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate<R> {
    Ref(R),
    And(Vec<Predicate<R>>),
    Or(Vec<Predicate<R>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateError {
    Empty,
}

impl std::fmt::Display for PredicateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("predicate must contain at least one element"),
        }
    }
}

impl std::error::Error for PredicateError {}

impl<R> Predicate<R> {
    pub fn and(predicates: Vec<Self>) -> Result<Self, PredicateError> {
        Self::non_empty(predicates).map(Self::And)
    }

    pub fn or(predicates: Vec<Self>) -> Result<Self, PredicateError> {
        Self::non_empty(predicates).map(Self::Or)
    }

    fn non_empty(predicates: Vec<Self>) -> Result<Vec<Self>, PredicateError> {
        if predicates.is_empty() {
            Err(PredicateError::Empty)
        } else {
            Ok(predicates)
        }
    }

    pub fn evaluate(&self, resolve: &mut impl FnMut(&R) -> bool) -> bool {
        match self {
            Self::Ref(reference) => resolve(reference),
            Self::And(predicates) => predicates
                .iter()
                .all(|predicate| predicate.evaluate(resolve)),
            Self::Or(predicates) => predicates
                .iter()
                .any(|predicate| predicate.evaluate(resolve)),
        }
    }

    pub fn validate<E>(&self, check: &mut impl FnMut(&R) -> Result<(), E>) -> Vec<E> {
        let mut errors = Vec::new();
        self.validate_into(check, &mut errors);
        errors
    }

    fn validate_into<E>(&self, check: &mut impl FnMut(&R) -> Result<(), E>, errors: &mut Vec<E>) {
        match self {
            Self::Ref(reference) => {
                if let Err(error) = check(reference) {
                    errors.push(error);
                }
            }
            Self::And(predicates) | Self::Or(predicates) => {
                for predicate in predicates {
                    predicate.validate_into(check, errors);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "predicate_test.rs"]
mod predicate_tests;

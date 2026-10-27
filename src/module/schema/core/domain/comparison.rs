use std::fmt;

/// What a comparison looked at. The categories are counted apart from one
/// another because a schema can come back with every table in place and
/// still have lost its keys, and one number over everything would hide that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Tables,
    Columns,
    Keys,
    ForeignKeys,
    Relationships,
    Descriptions,
    Positions,
}

impl Category {
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::Tables => "tables",
            Category::Columns => "columns",
            Category::Keys => "keys",
            Category::ForeignKeys => "foreign_keys",
            Category::Relationships => "relationships",
            Category::Descriptions => "descriptions",
            Category::Positions => "positions",
        }
    }

    pub const VARIANTS: [Category; 7] = [
        Category::Tables,
        Category::Columns,
        Category::Keys,
        Category::ForeignKeys,
        Category::Relationships,
        Category::Descriptions,
        Category::Positions,
    ];
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DifferenceKind {
    Missing,
    Extra,
    Changed,
}

impl DifferenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DifferenceKind::Missing => "missing",
            DifferenceKind::Extra => "extra",
            DifferenceKind::Changed => "changed",
        }
    }
}

impl fmt::Display for DifferenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing that did not survive, named by where it sits in the schema so
/// the reader can go straight to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub kind: DifferenceKind,
    pub path: String,
    pub expected: String,
    pub actual: String,
}

impl Difference {
    pub fn missing(path: impl Into<String>, expected: impl Into<String>) -> Self {
        Self {
            kind: DifferenceKind::Missing,
            path: path.into(),
            expected: expected.into(),
            actual: String::new(),
        }
    }

    pub fn extra(path: impl Into<String>, actual: impl Into<String>) -> Self {
        Self {
            kind: DifferenceKind::Extra,
            path: path.into(),
            expected: String::new(),
            actual: actual.into(),
        }
    }

    pub fn changed(
        path: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            kind: DifferenceKind::Changed,
            path: path.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub matched: u32,
    pub missing: u32,
    pub extra: u32,
    pub changed: u32,
}

impl Counts {
    /// Of what came back, how much was right. Something that came back wrong
    /// counts against this as much as something invented, which is why a
    /// change sits on both sides of the ratio.
    pub fn precision(&self) -> f64 {
        ratio(self.matched, self.matched + self.extra + self.changed)
    }

    /// Of what was expected, how much came back.
    pub fn recall(&self) -> f64 {
        ratio(self.matched, self.matched + self.missing + self.changed)
    }

    pub fn is_clean(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.changed == 0
    }

    pub fn count(&mut self, kind: DifferenceKind) {
        match kind {
            DifferenceKind::Missing => self.missing += 1,
            DifferenceKind::Extra => self.extra += 1,
            DifferenceKind::Changed => self.changed += 1,
        }
    }
}

/// Nothing to find and nothing got wrong is a perfect score rather than a
/// zero, so a schema with no foreign keys does not read as having lost them.
fn ratio(matched: u32, total: u32) -> f64 {
    match total {
        0 => 1.0,
        _ => f64::from(matched) / f64::from(total),
    }
}

/// How much of one schema came back in another. Produced by comparing a
/// drawn schema against the one a round trip through SQL or JSON returned,
/// which turns "it round-trips" into a number.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comparison {
    pub differences: Vec<Difference>,
    pub tables: Counts,
    pub columns: Counts,
    pub keys: Counts,
    pub foreign_keys: Counts,
    pub relationships: Counts,
    pub descriptions: Counts,
    pub positions: Counts,
}

impl Comparison {
    pub fn is_equal(&self) -> bool {
        Category::VARIANTS
            .iter()
            .all(|category| self.counts(*category).is_clean())
    }

    pub fn counts(&self, category: Category) -> Counts {
        match category {
            Category::Tables => self.tables,
            Category::Columns => self.columns,
            Category::Keys => self.keys,
            Category::ForeignKeys => self.foreign_keys,
            Category::Relationships => self.relationships,
            Category::Descriptions => self.descriptions,
            Category::Positions => self.positions,
        }
    }

    pub fn counts_mut(&mut self, category: Category) -> &mut Counts {
        match category {
            Category::Tables => &mut self.tables,
            Category::Columns => &mut self.columns,
            Category::Keys => &mut self.keys,
            Category::ForeignKeys => &mut self.foreign_keys,
            Category::Relationships => &mut self.relationships,
            Category::Descriptions => &mut self.descriptions,
            Category::Positions => &mut self.positions,
        }
    }

    pub fn record(&mut self, category: Category, difference: Difference) {
        self.counts_mut(category).count(difference.kind);
        self.differences.push(difference);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_compare_scores_a_clean_one() {
        let counts = Counts::default();

        assert_eq!(counts.precision(), 1.0);
        assert_eq!(counts.recall(), 1.0);
        assert!(counts.is_clean());
    }

    #[test]
    fn something_that_came_back_wrong_costs_both_scores() {
        let counts = Counts {
            matched: 3,
            missing: 0,
            extra: 0,
            changed: 1,
        };

        assert_eq!(counts.precision(), 0.75);
        assert_eq!(counts.recall(), 0.75);
    }

    #[test]
    fn what_was_lost_and_what_was_invented_cost_different_scores() {
        let lost = Counts {
            matched: 1,
            missing: 1,
            extra: 0,
            changed: 0,
        };

        assert_eq!(lost.recall(), 0.5);
        assert_eq!(lost.precision(), 1.0);

        let invented = Counts {
            matched: 1,
            missing: 0,
            extra: 1,
            changed: 0,
        };

        assert_eq!(invented.recall(), 1.0);
        assert_eq!(invented.precision(), 0.5);
    }

    #[test]
    fn a_comparison_is_equal_until_something_is_recorded_against_it() {
        let mut comparison = Comparison::default();

        assert!(comparison.is_equal());

        comparison.record(
            Category::Columns,
            Difference::missing("entities[users].attributes[email]", "email"),
        );

        assert!(!comparison.is_equal());
        assert_eq!(comparison.columns.missing, 1);
        assert_eq!(comparison.differences.len(), 1);
    }

    #[test]
    fn every_category_is_reachable_by_name() {
        for category in Category::VARIANTS {
            let mut comparison = Comparison::default();

            comparison.counts_mut(category).matched += 1;

            assert_eq!(comparison.counts(category).matched, 1, "{category}");
        }
    }
}

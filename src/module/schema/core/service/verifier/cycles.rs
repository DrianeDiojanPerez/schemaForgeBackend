use std::collections::HashMap;

use crate::module::schema::core::domain::{diagnostic_code as code, Diagnostic, Report, Schema};

use super::table;

/// A table that references itself is left out. A category with a parent
/// category is normal, and a row can always point at itself.
///
/// When every foreign key around a cycle is NOT NULL, no table in it can take
/// the first row, so that is an error. If one of them is nullable the rows can
/// go in with a NULL and be filled in later, so it is only a warning.
pub(super) fn check_cycles(schema: &Schema, report: &mut Report) {
    let index: HashMap<&str, usize> = schema
        .entities
        .iter()
        .enumerate()
        .map(|(position, entity)| (entity.id.as_str(), position))
        .collect();

    let mut all = Vec::new();
    let mut required = Vec::new();

    for (from, entity) in schema.entities.iter().enumerate() {
        for attribute in &entity.attributes {
            let Some(reference) = &attribute.foreign_key else {
                continue;
            };

            let Some(&to) = index.get(reference.entity_id.as_str()) else {
                continue;
            };

            if to == from {
                continue;
            }

            all.push((from, to));

            if !attribute.nullable {
                required.push((from, to));
            }
        }
    }

    let blocked = strongly_connected(schema.entities.len(), &required);

    for component in strongly_connected(schema.entities.len(), &all) {
        let names = component
            .iter()
            .map(|&position| table(&schema.entities[position]))
            .collect::<Vec<_>>()
            .join(", ");

        let is_blocked = blocked
            .iter()
            .any(|cycle| cycle.iter().any(|position| component.contains(position)));

        let mut diagnostic = if is_blocked {
            Diagnostic::error(
                code::CIRCULAR_DEPENDENCY,
                format!(
                    "{names} depend on each other through foreign keys that are all NOT NULL, so no row can be inserted first"
                ),
            )
        } else {
            Diagnostic::warning(
                code::CIRCULAR_DEPENDENCY,
                format!(
                    "{names} depend on each other through foreign keys, so some rows have to be inserted with a NULL and updated later"
                ),
            )
        }
        .at(schema.entities[component[0]].position);

        for &position in &component {
            diagnostic = diagnostic.about(&schema.entities[position].id);
        }

        report.push(diagnostic);
    }
}

struct Tarjan<'a> {
    adjacency: &'a [Vec<usize>],
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    components: Vec<Vec<usize>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, node: usize) {
        self.index[node] = Some(self.next);
        self.low[node] = self.next;
        self.next += 1;
        self.stack.push(node);
        self.on_stack[node] = true;

        for &neighbour in self.adjacency[node].iter() {
            match self.index[neighbour] {
                None => {
                    self.visit(neighbour);
                    self.low[node] = self.low[node].min(self.low[neighbour]);
                }
                Some(index) if self.on_stack[neighbour] => {
                    self.low[node] = self.low[node].min(index);
                }
                Some(_) => {}
            }
        }

        if Some(self.low[node]) != self.index[node] {
            return;
        }

        let mut component = Vec::new();

        while let Some(member) = self.stack.pop() {
            self.on_stack[member] = false;
            component.push(member);

            if member == node {
                break;
            }
        }

        self.components.push(component);
    }
}

/// Groups of two or more tables that can each reach the others, sorted so
/// the report reads in the same order as the schema.
fn strongly_connected(count: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::new(); count];

    for &(from, to) in edges {
        adjacency[from].push(to);
    }

    let mut tarjan = Tarjan {
        adjacency: &adjacency,
        index: vec![None; count],
        low: vec![0; count],
        on_stack: vec![false; count],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };

    for node in 0..count {
        if tarjan.index[node].is_none() {
            tarjan.visit(node);
        }
    }

    let mut components: Vec<Vec<usize>> = tarjan
        .components
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|mut component| {
            component.sort_unstable();
            component
        })
        .collect();

    components.sort_unstable();
    components
}

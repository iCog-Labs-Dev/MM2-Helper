use mork_expr::{byte_item, Expr, Tag};
use pathmap::zipper::{ZipperIteration, ZipperMoving};
use pathmap::PathMap;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

#[derive(Clone, Debug)]
enum PatternTerm {
    Variable(usize),
    Symbol(Vec<u8>),
    Expression(Vec<PatternTerm>),
}

#[derive(Clone, Debug)]
enum DataTerm {
    Variable(Vec<u8>),
    Symbol {
        encoded: Vec<u8>,
        value: Vec<u8>,
    },
    Expression {
        encoded: Vec<u8>,
        children: Vec<DataTerm>,
        contains_variable: bool,
    },
}

impl DataTerm {
    fn encoded(&self) -> &[u8] {
        match self {
            Self::Variable(encoded)
            | Self::Symbol { encoded, .. }
            | Self::Expression { encoded, .. } => encoded,
        }
    }

    fn contains_variable(&self) -> bool {
        match self {
            Self::Variable(_) => true,
            Self::Symbol { .. } => false,
            Self::Expression {
                contains_variable, ..
            } => *contains_variable,
        }
    }
}

#[derive(Clone, Debug)]
struct Factor {
    variables: Vec<usize>,
    rows: HashMap<Vec<Vec<u8>>, u128>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatchResult {
    Matched,
    NoMatch,
    Unsupported,
}

fn parse_pattern(
    bytes: &[u8],
    offset: &mut usize,
    next_variable: &mut usize,
) -> Option<PatternTerm> {
    let start = *offset;
    let tag = byte_item(*bytes.get(*offset)?);
    *offset += 1;
    match tag {
        Tag::NewVar => {
            let variable = *next_variable;
            *next_variable += 1;
            Some(PatternTerm::Variable(variable))
        }
        Tag::VarRef(variable) => Some(PatternTerm::Variable(variable as usize)),
        Tag::SymbolSize(size) => {
            let end = offset.checked_add(size as usize)?;
            let value = bytes.get(*offset..end)?.to_vec();
            *offset = end;
            Some(PatternTerm::Symbol(value))
        }
        Tag::Arity(arity) => {
            let mut children = Vec::with_capacity(arity as usize);
            for _ in 0..arity {
                children.push(parse_pattern(bytes, offset, next_variable)?);
            }
            debug_assert!(*offset > start);
            Some(PatternTerm::Expression(children))
        }
    }
}

fn parse_data(bytes: &[u8], offset: &mut usize) -> Option<DataTerm> {
    let start = *offset;
    let tag = byte_item(*bytes.get(*offset)?);
    *offset += 1;
    match tag {
        Tag::NewVar | Tag::VarRef(_) => {
            Some(DataTerm::Variable(bytes.get(start..*offset)?.to_vec()))
        }
        Tag::SymbolSize(size) => {
            let end = offset.checked_add(size as usize)?;
            let value = bytes.get(*offset..end)?.to_vec();
            *offset = end;
            Some(DataTerm::Symbol {
                encoded: bytes.get(start..end)?.to_vec(),
                value,
            })
        }
        Tag::Arity(arity) => {
            let mut children = Vec::with_capacity(arity as usize);
            let mut contains_variable = false;
            for _ in 0..arity {
                let child = parse_data(bytes, offset)?;
                contains_variable |= child.contains_variable();
                children.push(child);
            }
            Some(DataTerm::Expression {
                encoded: bytes.get(start..*offset)?.to_vec(),
                children,
                contains_variable,
            })
        }
    }
}

fn collect_variables(pattern: &PatternTerm, variables: &mut BTreeSet<usize>) {
    match pattern {
        PatternTerm::Variable(variable) => {
            variables.insert(*variable);
        }
        PatternTerm::Expression(children) => {
            for child in children {
                collect_variables(child, variables);
            }
        }
        PatternTerm::Symbol(_) => {}
    }
}

fn match_pattern(
    pattern: &PatternTerm,
    data: &DataTerm,
    bindings: &mut HashMap<usize, Vec<u8>>,
) -> MatchResult {
    match pattern {
        PatternTerm::Variable(variable) => {
            if data.contains_variable() {
                return MatchResult::Unsupported;
            }
            match bindings.get(variable) {
                Some(bound) if bound.as_slice() != data.encoded() => MatchResult::NoMatch,
                Some(_) => MatchResult::Matched,
                None => {
                    bindings.insert(*variable, data.encoded().to_vec());
                    MatchResult::Matched
                }
            }
        }
        PatternTerm::Symbol(expected) => match data {
            DataTerm::Symbol { value, .. } if value == expected => MatchResult::Matched,
            _ => MatchResult::NoMatch,
        },
        PatternTerm::Expression(expected) => match data {
            DataTerm::Expression { children, .. } if children.len() == expected.len() => {
                for (pattern_child, data_child) in expected.iter().zip(children) {
                    match match_pattern(pattern_child, data_child, bindings) {
                        MatchResult::Matched => {}
                        other => return other,
                    }
                }
                MatchResult::Matched
            }
            _ => MatchResult::NoMatch,
        },
    }
}

fn build_factor(space: &PathMap<()>, pattern: &PatternTerm) -> Option<Factor> {
    let mut variable_set = BTreeSet::new();
    collect_variables(pattern, &mut variable_set);
    let variables = variable_set.into_iter().collect::<Vec<_>>();
    let mut rows = HashMap::<Vec<Vec<u8>>, u128>::new();
    let mut zipper = space.read_zipper();

    while zipper.to_next_val() {
        let path = zipper.path();
        let mut offset = 0;
        let Some(data) = parse_data(path, &mut offset) else {
            return None;
        };
        if offset != path.len() {
            return None;
        }

        let mut bindings = HashMap::new();
        match match_pattern(pattern, &data, &mut bindings) {
            MatchResult::Matched => {
                let row = variables
                    .iter()
                    .map(|variable| bindings.get(variable).cloned())
                    .collect::<Option<Vec<_>>>()?;
                let entry = rows.entry(row).or_default();
                *entry = entry.checked_add(1)?;
            }
            MatchResult::NoMatch => {}
            MatchResult::Unsupported => return None,
        }
    }

    Some(Factor { variables, rows })
}

fn join(left: Factor, right: Factor) -> Option<Factor> {
    let left_positions = left
        .variables
        .iter()
        .enumerate()
        .map(|(index, variable)| (*variable, index))
        .collect::<HashMap<_, _>>();
    let right_positions = right
        .variables
        .iter()
        .enumerate()
        .map(|(index, variable)| (*variable, index))
        .collect::<HashMap<_, _>>();
    let shared = left
        .variables
        .iter()
        .filter(|variable| right_positions.contains_key(variable))
        .copied()
        .collect::<Vec<_>>();
    let right_extra = right
        .variables
        .iter()
        .filter(|variable| !left_positions.contains_key(variable))
        .copied()
        .collect::<Vec<_>>();

    let mut variables = left.variables.clone();
    variables.extend(right_extra.iter().copied());

    let mut index = HashMap::<Vec<Vec<u8>>, Vec<(Vec<Vec<u8>>, u128)>>::new();
    for (row, weight) in right.rows {
        let key = shared
            .iter()
            .map(|variable| row[right_positions[variable]].clone())
            .collect::<Vec<_>>();
        index.entry(key).or_default().push((row, weight));
    }

    let mut rows = HashMap::<Vec<Vec<u8>>, u128>::new();
    for (left_row, left_weight) in left.rows {
        let key = shared
            .iter()
            .map(|variable| left_row[left_positions[variable]].clone())
            .collect::<Vec<_>>();
        let Some(matches) = index.get(&key) else {
            continue;
        };
        for (right_row, right_weight) in matches {
            let mut output = left_row.clone();
            output.extend(
                right_extra
                    .iter()
                    .map(|variable| right_row[right_positions[variable]].clone()),
            );
            let product = left_weight.checked_mul(*right_weight)?;
            let entry = rows.entry(output).or_default();
            *entry = entry.checked_add(product)?;
        }
    }

    Some(Factor { variables, rows })
}

fn eliminate(factor: Factor, variable: usize) -> Option<Factor> {
    let position = factor.variables.iter().position(|item| *item == variable)?;
    let mut variables = factor.variables;
    variables.remove(position);
    let mut rows = HashMap::<Vec<Vec<u8>>, u128>::new();
    for (mut row, weight) in factor.rows {
        row.remove(position);
        let entry = rows.entry(row).or_default();
        *entry = entry.checked_add(weight)?;
    }
    Some(Factor { variables, rows })
}

fn choose_variable(factors: &[Factor], variables: &HashSet<usize>) -> Option<usize> {
    variables.iter().copied().min_by_key(|variable| {
        let involved = factors
            .iter()
            .filter(|factor| factor.variables.contains(variable))
            .collect::<Vec<_>>();
        let neighbors = involved
            .iter()
            .flat_map(|factor| factor.variables.iter().copied())
            .filter(|neighbor| neighbor != variable)
            .collect::<HashSet<_>>();
        let input_rows = involved
            .iter()
            .map(|factor| factor.rows.len())
            .sum::<usize>();
        (neighbors.len(), input_rows, *variable)
    })
}

/// Count a conjunction through grouped variable elimination.
///
/// Returns `None` when the query uses a shape that cannot be evaluated without
/// changing MORK's normal unification semantics. Callers must then fall back to
/// the regular streaming matcher.
pub(crate) fn count(space: &PathMap<()>, query: Expr) -> Option<usize> {
    let started = Instant::now();
    let trace = std::env::var_os("MM2_FACTORIZED_TRACE").is_some();
    let bytes = unsafe { query.span().as_ref()? };
    let mut offset = 0;
    let mut next_variable = 0;
    let PatternTerm::Expression(mut query_items) =
        parse_pattern(bytes, &mut offset, &mut next_variable)?
    else {
        return None;
    };
    if offset != bytes.len() || query_items.is_empty() {
        return None;
    }
    match query_items.remove(0) {
        PatternTerm::Symbol(head) if head == b"," => {}
        _ => return None,
    }
    if query_items.is_empty() {
        return Some(1);
    }
    if trace {
        eprintln!("count-factorized start: {query_items:?}");
    }

    let mut factors = query_items
        .iter()
        .map(|pattern| build_factor(space, pattern))
        .collect::<Option<Vec<_>>>()?;
    let mut variables = factors
        .iter()
        .flat_map(|factor| factor.variables.iter().copied())
        .collect::<HashSet<_>>();

    while let Some(variable) = choose_variable(&factors, &variables) {
        let mut involved = Vec::new();
        let mut remaining = Vec::new();
        for factor in factors {
            if factor.variables.contains(&variable) {
                involved.push(factor);
            } else {
                remaining.push(factor);
            }
        }
        let mut combined = involved.pop()?;
        for factor in involved {
            combined = join(combined, factor)?;
        }
        remaining.push(eliminate(combined, variable)?);
        factors = remaining;
        variables.remove(&variable);
    }

    let mut result = 1_u128;
    for factor in factors {
        result = result.checked_mul(*factor.rows.get(&Vec::new()).unwrap_or(&0))?;
    }
    let result = usize::try_from(result).ok();
    if trace {
        eprintln!(
            "count-factorized finish: result={result:?} elapsed={:?}",
            started.elapsed()
        );
    }
    result
}

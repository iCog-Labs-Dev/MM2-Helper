use eval::{EvalScope, FuncType};
use eval_ffi::{EvalError, ExprSink, ExprSource, Tag};
use mork_expr::{item_byte, Expr, ExprEnv, ExprZipper, SourceItem};
use crate::sinks::{Sink, WriteResource, WriteResourceRequest};
use pathmap::zipper::{WriteZipperTracked, ZipperAbsolutePath, ZipperMoving, ZipperWriting};
use std::collections::{HashMap, HashSet};

fn expr_span(e: Expr) -> &'static [u8] {
    unsafe { e.span().as_ref().unwrap() }
}

fn consume_named_expr_1(expr: &mut ExprSource, name: &[u8]) -> Result<Expr, EvalError> {
    let items = expr.consume_head_check(name)?;
    if items != 1 {
        return Err(EvalError::from("takes one argument"));
    }
    expr.consume::<Expr>()
}

fn tuple_items(tuple_expr: Expr) -> Result<Vec<Expr>, EvalError> {
    match mork_expr::byte_item(unsafe { *tuple_expr.ptr }) {
        Tag::Arity(_) => {
            let mut env_items = Vec::new();
            ExprEnv::new(0, tuple_expr).args(&mut env_items);
            Ok(env_items.into_iter().map(|e| e.subsexpr()).collect())
        }
        _ => Err(EvalError::from("expects a tuple/expression argument")),
    }
}

fn expr_symbol_bytes(e: Expr) -> Result<&'static [u8], EvalError> {
    unsafe {
        let Tag::SymbolSize(size) = mork_expr::byte_item(*e.ptr) else {
            return Err(EvalError::from("expected symbol"));
        };
        Ok(std::slice::from_raw_parts(e.ptr.add(1), size as usize))
    }
}

fn write_normalized_expr(sink: &mut ExprSink, mut bytes: Vec<u8>) -> Result<(), EvalError> {
    let mut out = vec![0u8; bytes.len()];
    let mut ez = ExprZipper::new(Expr {
        ptr: bytes.as_mut_ptr(),
    });
    let mut oz = ExprZipper::new(Expr {
        ptr: out.as_mut_ptr(),
    });
    let mut var_map = [None; 64];
    let mut input_new_vars = 0usize;
    let mut output_new_vars = 0u8;

    loop {
        match ez.tag() {
            Tag::NewVar => {
                if input_new_vars >= var_map.len() {
                    return Err(EvalError::from("too many variables in expression"));
                }
                if output_new_vars >= 64 {
                    return Err(EvalError::from("too many variables in expression"));
                }

                var_map[input_new_vars] = Some(output_new_vars);
                oz.write_new_var();
                oz.loc += 1;
                input_new_vars += 1;
                output_new_vars += 1;
            }
            Tag::VarRef(i) => {
                let mapped = match var_map[i as usize] {
                    Some(mapped) => mapped,
                    None => {
                        if output_new_vars >= 64 {
                            return Err(EvalError::from("too many variables in expression"));
                        }

                        let mapped = output_new_vars;
                        var_map[i as usize] = Some(mapped);
                        output_new_vars += 1;
                        oz.write_new_var();
                        oz.loc += 1;
                        if !ez.next() {
                            break;
                        }
                        continue;
                    }
                };

                oz.write_var_ref(mapped);
                oz.loc += 1;
            }
            Tag::SymbolSize(s) => {
                let symbol = unsafe {
                    std::slice::from_raw_parts(ez.root.ptr.byte_add(ez.loc), s as usize + 1)
                };
                oz.write_move(symbol);
            }
            Tag::Arity(_) => {
                unsafe {
                    *oz.root.ptr.byte_add(oz.loc) = *ez.root.ptr.byte_add(ez.loc);
                }
                oz.loc += 1;
            }
        }

        if !ez.next() {
            break;
        }
    }

    sink.extend_from_slice(&out[..oz.loc])?;
    Ok(())
}

fn push_tuple_from_items(out: &mut Vec<u8>, items: &[Expr]) -> Result<(), EvalError> {
    if items.len() > u8::MAX as usize {
        return Err(EvalError::from("tuple arity exceeds 255"));
    }

    out.push(item_byte(Tag::Arity(items.len() as u8)));
    for e in items {
        out.extend_from_slice(expr_span(*e));
    }
    Ok(())
}

fn write_var_marker(sink: &mut ExprSink, index: usize) -> Result<(), EvalError> {
    let index = index.to_string();
    sink.write(SourceItem::Tag(Tag::Arity(2)))?;
    sink.write(SourceItem::Symbol(b"var"))?;
    sink.write(SourceItem::Symbol(index.as_bytes()))?;
    Ok(())
}

fn collect_indexed_vars(
    e: Expr,
    seen: &mut HashSet<Vec<u8>>,
    vars: &mut Vec<Vec<u8>>,
) -> Result<(), EvalError> {
    if var_marker_key(e)?.is_some() {
        let bytes = expr_span(e).to_vec();
        if seen.insert(bytes.clone()) {
            vars.push(bytes);
        }
        return Ok(());
    }

    unsafe {
        if let Tag::Arity(arity) = mork_expr::byte_item(*e.ptr) {
            let mut offset = 1usize;
            for _ in 0..arity {
                let child = Expr {
                    ptr: e.ptr.add(offset),
                };
                collect_indexed_vars(child, seen, vars)?;
                offset += expr_span(child).len();
            }
        }
    }

    Ok(())
}

fn var_marker_key(e: Expr) -> Result<Option<Vec<u8>>, EvalError> {
    unsafe {
        let Tag::Arity(2) = mork_expr::byte_item(*e.ptr) else {
            return Ok(None);
        };

        let mut offset = 1usize;
        let Tag::SymbolSize(head_len) = mork_expr::byte_item(*e.ptr.add(offset)) else {
            return Ok(None);
        };
        offset += 1;
        let head = std::slice::from_raw_parts(e.ptr.add(offset), head_len as usize);
        if head != b"var" {
            return Ok(None);
        }
        offset += head_len as usize;

        let key = Expr {
            ptr: e.ptr.add(offset),
        };
        Ok(Some(expr_span(key).to_vec()))
    }
}

fn write_indices_as_vars(
    e: Expr,
    sink: &mut ExprSink,
    labels: &mut HashMap<Vec<u8>, u8>,
    introduced: &mut u8,
) -> Result<(), EvalError> {
    if let Some(key) = var_marker_key(e)? {
        if let Some(index) = labels.get(&key) {
            sink.write(SourceItem::Tag(Tag::VarRef(*index)))?;
        } else {
            if *introduced >= 64 {
                return Err(EvalError::from("can only introduce up to 64 variables"));
            }
            let index = *introduced;
            labels.insert(key, index);
            sink.write(SourceItem::Tag(Tag::NewVar))?;
            *introduced += 1;
        }
        return Ok(());
    }

    unsafe {
        match mork_expr::byte_item(*e.ptr) {
            Tag::NewVar => {
                if *introduced >= 64 {
                    return Err(EvalError::from("can only introduce up to 64 variables"));
                }
                sink.write(SourceItem::Tag(Tag::NewVar))?;
                *introduced += 1;
            }
            Tag::VarRef(i) => {
                sink.write(SourceItem::Tag(Tag::VarRef(i)))?;
            }
            Tag::SymbolSize(size) => {
                let symbol = std::slice::from_raw_parts(e.ptr.add(1), size as usize);
                sink.write(SourceItem::Symbol(symbol))?;
            }
            Tag::Arity(arity) => {
                sink.write(SourceItem::Tag(Tag::Arity(arity)))?;
                let mut offset = 1usize;
                for _ in 0..arity {
                    let child = Expr {
                        ptr: e.ptr.add(offset),
                    };
                    write_indices_as_vars(child, sink, labels, introduced)?;
                    offset += expr_span(child).len();
                }
            }
        }
    }

    Ok(())
}

struct SubstitutionState {
    next_value: usize,
}

impl SubstitutionState {
    fn new() -> Self {
        Self { next_value: 0 }
    }

    fn write_replacement_or_original(
        &mut self,
        original: Expr,
        sink: &mut ExprSink,
        values: &[Expr],
    ) -> Result<(), EvalError> {
        if self.next_value >= values.len() {
            write_expr(sink, original)?;
        } else {
            let replacement = values[self.next_value];
            self.next_value += 1;
            sink.extend_from_slice(expr_span(replacement))?;
        }

        Ok(())
    }
}

fn write_substituted_expr(
    e: Expr,
    sink: &mut ExprSink,
    values: &[Expr],
    state: &mut SubstitutionState,
) -> Result<(), EvalError> {
    if state.next_value >= values.len() {
        return write_expr(sink, e);
    }

    if var_marker_key(e)?.is_some() {
        return state.write_replacement_or_original(e, sink, values);
    }

    unsafe {
        match mork_expr::byte_item(*e.ptr) {
            Tag::NewVar | Tag::VarRef(_) => {
                state.write_replacement_or_original(e, sink, values)?;
            }
            Tag::SymbolSize(size) => {
                let symbol = std::slice::from_raw_parts(e.ptr.add(1), size as usize);
                sink.write(SourceItem::Symbol(symbol))?;
            }
            Tag::Arity(arity) => {
                sink.write(SourceItem::Tag(Tag::Arity(arity)))?;
                let mut offset = 1usize;
                for _ in 0..arity {
                    let child = Expr {
                        ptr: e.ptr.add(offset),
                    };
                    write_substituted_expr(child, sink, values, state)?;
                    offset += expr_span(child).len();
                }
            }
        }
    }

    Ok(())
}

fn substitute_args(expr: &mut ExprSource) -> Result<(Expr, Expr), EvalError> {
    let items = expr.consume_head_check(b"substitute")?;
    match items {
        1 => {
            let pair = expr.consume::<Expr>()?;
            let pair_items = tuple_items(pair)?;
            if pair_items.len() != 2 {
                return Err(EvalError::from("substitute pair must be (pattern values)"));
            }
            Ok((pair_items[0], pair_items[1]))
        }
        2 => {
            let values = expr.consume::<Expr>()?;
            let pattern = expr.consume::<Expr>()?;
            Ok((pattern, values))
        }
        _ => Err(EvalError::from(
            "substitute takes either one pair or two arguments",
        )),
    }
}

fn write_expr(sink: &mut ExprSink, expr: Expr) -> Result<(), EvalError> {
    write_normalized_expr(sink, expr_span(expr).to_vec())
}

fn partition_key(partition: &[Vec<Expr>]) -> Vec<Vec<Vec<u8>>> {
    partition
        .iter()
        .map(|block| block.iter().map(|e| expr_span(*e).to_vec()).collect())
        .collect()
}

fn build_partitions(
    items: &[Expr],
    index: usize,
    blocks: &mut Vec<Vec<Expr>>,
    out: &mut Vec<Vec<Vec<Expr>>>,
    seen: &mut HashSet<Vec<Vec<Vec<u8>>>>,
) {
    if index == items.len() {
        if blocks.len() <= 1 {
            return;
        }

        let key = partition_key(blocks);
        if seen.insert(key) {
            out.push(blocks.clone());
        }
        return;
    }

    for block_index in 0..blocks.len() {
        blocks[block_index].push(items[index]);
        build_partitions(items, index + 1, blocks, out, seen);
        blocks[block_index].pop();
    }

    blocks.push(vec![items[index]]);
    build_partitions(items, index + 1, blocks, out, seen);
    blocks.pop();
}

fn write_partitions(sink: &mut ExprSink, partitions: &[Vec<Vec<Expr>>]) -> Result<(), EvalError> {
    if partitions.len() > u8::MAX as usize {
        return Err(EvalError::from("tuple arity exceeds 255"));
    }

    let mut out = Vec::new();
    out.push(item_byte(Tag::Arity(partitions.len() as u8)));
    for partition in partitions {
        if partition.len() > u8::MAX as usize {
            return Err(EvalError::from("tuple arity exceeds 255"));
        }

        out.push(item_byte(Tag::Arity(partition.len() as u8)));
        for block in partition {
            push_tuple_from_items(&mut out, block)?;
        }
    }

    write_normalized_expr(sink, out)
}

fn factorial_i64(n: i64) -> Result<i64, EvalError> {
    if n < 0 {
        return Err(EvalError::from("factorial expects n >= 0"));
    }

    let mut result = 1i64;
    for i in 2..=n {
        result = result
            .checked_mul(i)
            .ok_or_else(|| EvalError::from("factorial overflow"))?;
    }
    Ok(result)
}

fn falling_factorial_i64(n: i64, k: i64) -> Result<i64, EvalError> {
    if n < 0 || k < 0 {
        return Err(EvalError::from(
            "falling_factorial expects n >= 0 and k >= 0",
        ));
    }
    if k > n {
        return Err(EvalError::from("falling_factorial expects k <= n"));
    }

    let mut result = 1i64;
    for i in 0..k {
        result = result
            .checked_mul(n - i)
            .ok_or_else(|| EvalError::from("falling_factorial overflow"))?;
    }
    Ok(result)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum PatternVarKey {
    Raw(usize),
    Marker(Vec<u8>),
}

struct SubsumesState {
    raw_new_vars: usize,
    bindings: HashMap<PatternVarKey, Vec<u8>>,
}

impl SubsumesState {
    fn new() -> Self {
        Self {
            raw_new_vars: 0,
            bindings: HashMap::new(),
        }
    }

    fn raw_key(&mut self, tag: Tag) -> Option<PatternVarKey> {
        match tag {
            Tag::NewVar => {
                let key = PatternVarKey::Raw(self.raw_new_vars);
                self.raw_new_vars += 1;
                Some(key)
            }
            Tag::VarRef(i) => Some(PatternVarKey::Raw(i as usize)),
            _ => None,
        }
    }

    fn bind_or_compare(&mut self, key: PatternVarKey, target: Expr) -> bool {
        let target_span = expr_span(target);
        if let Some(bound) = self.bindings.get(&key) {
            bound.as_slice() == target_span
        } else {
            self.bindings.insert(key, target_span.to_vec());
            true
        }
    }
}

fn subsumes_expr(pattern: Expr, target: Expr, state: &mut SubsumesState) -> Result<bool, EvalError> {
    if let Some(key) = var_marker_key(pattern)? {
        return Ok(state.bind_or_compare(PatternVarKey::Marker(key), target));
    }

    unsafe {
        let pattern_tag = mork_expr::byte_item(*pattern.ptr);
        if let Some(key) = state.raw_key(pattern_tag) {
            return Ok(state.bind_or_compare(key, target));
        }

        match (pattern_tag, mork_expr::byte_item(*target.ptr)) {
            (Tag::SymbolSize(pattern_size), Tag::SymbolSize(target_size)) => {
                if pattern_size != target_size {
                    return Ok(false);
                }

                let pattern_symbol = std::slice::from_raw_parts(pattern.ptr.add(1), pattern_size as usize);
                let target_symbol = std::slice::from_raw_parts(target.ptr.add(1), target_size as usize);
                Ok(pattern_symbol == target_symbol)
            }
            (Tag::Arity(pattern_arity), Tag::Arity(target_arity)) => {
                if pattern_arity != target_arity {
                    return Ok(false);
                }

                let mut pattern_offset = 1usize;
                let mut target_offset = 1usize;
                for _ in 0..pattern_arity {
                    let pattern_child = Expr {
                        ptr: pattern.ptr.add(pattern_offset),
                    };
                    let target_child = Expr {
                        ptr: target.ptr.add(target_offset),
                    };

                    if !subsumes_expr(pattern_child, target_child, state)? {
                        return Ok(false);
                    }

                    pattern_offset += expr_span(pattern_child).len();
                    target_offset += expr_span(target_child).len();
                }

                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn strictly_subsumes_expr(pattern: Expr, target: Expr) -> Result<bool, EvalError> {
    let mut forward_state = SubsumesState::new();
    if !subsumes_expr(pattern, target, &mut forward_state)? {
        return Ok(false);
    }

    let mut reverse_state = SubsumesState::new();
    Ok(!subsumes_expr(target, pattern, &mut reverse_state)?)
}

fn is_subsumed_by_expr(pattern: Expr, other: Expr) -> Result<bool, EvalError> {
    let mut state = SubsumesState::new();
    subsumes_expr(other, pattern, &mut state)
}

fn is_strictly_subsumed_by_expr(pattern: Expr, other: Expr) -> Result<bool, EvalError> {
    strictly_subsumes_expr(other, pattern)
}

fn not_subsumed_expr(pattern: Expr, other: Expr) -> Result<bool, EvalError> {
    Ok(!is_strictly_subsumed_by_expr(pattern, other)?)
}

fn subsumes_args(expr: &mut ExprSource, name: &[u8]) -> Result<(Expr, Expr), EvalError> {
    let items = expr.consume_head_check(name)?;
    match items {
        1 => {
            let pair = expr.consume::<Expr>()?;
            let pair_items = tuple_items(pair)?;
            if pair_items.len() != 2 {
                return Err(EvalError::from("subsumes pair must be (pattern target)"));
            }
            Ok((pair_items[0], pair_items[1]))
        }
        2 => {
            let target = expr.consume::<Expr>()?;
            let pattern = expr.consume::<Expr>()?;
            Ok((pattern, target))
        }
        _ => Err(EvalError::from("subsumes takes either one pair or two arguments")),
    }
}

fn write_bool(sink: &mut ExprSink, value: bool) -> Result<(), EvalError> {
    let value = [u8::from(value)];
    sink.write(SourceItem::Symbol(&value))?;
    Ok(())
}

fn ln_gamma(x: f64) -> f64 {
    const COEFFS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];

    if x < 0.5 {
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * x).sin().ln()
            - ln_gamma(1.0 - x);
    }

    let z = x - 1.0;
    let mut a = COEFFS[0];
    for (i, coeff) in COEFFS.iter().enumerate().skip(1) {
        a += coeff / (z + i as f64);
    }
    let t = z + 7.5;

    0.5 * (2.0 * std::f64::consts::PI).ln() + (z + 0.5) * t.ln() - t + a.ln()
}

fn beta_continued_fraction(a: f64, b: f64, x: f64) -> Result<f64, EvalError> {
    const MAX_ITERATIONS: usize = 200;
    const EPSILON: f64 = 3.0e-14;
    const MIN_FLOAT: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < MIN_FLOAT {
        d = MIN_FLOAT;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=MAX_ITERATIONS {
        let m_f = m as f64;
        let m2 = 2.0 * m_f;

        let mut aa = m_f * (b - m_f) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < MIN_FLOAT {
            d = MIN_FLOAT;
        }
        c = 1.0 + aa / c;
        if c.abs() < MIN_FLOAT {
            c = MIN_FLOAT;
        }
        d = 1.0 / d;
        h *= d * c;

        aa = -(a + m_f) * (qab + m_f) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < MIN_FLOAT {
            d = MIN_FLOAT;
        }
        c = 1.0 + aa / c;
        if c.abs() < MIN_FLOAT {
            c = MIN_FLOAT;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;

        if (delta - 1.0).abs() < EPSILON {
            return Ok(h);
        }
    }

    Err(EvalError::from(
        "beta_cdf_f64 continued fraction did not converge",
    ))
}

fn regularized_beta_cdf(a: f64, b: f64, x: f64) -> Result<f64, EvalError> {
    if !a.is_finite() || !b.is_finite() || !x.is_finite() {
        return Err(EvalError::from("beta_cdf_f64 expects finite arguments"));
    }
    if a <= 0.0 || b <= 0.0 {
        return Err(EvalError::from(
            "beta_cdf_f64 expects alpha > 0 and beta > 0",
        ));
    }
    if !(0.0..=1.0).contains(&x) {
        return Err(EvalError::from("beta_cdf_f64 expects 0 <= x <= 1"));
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if x == 1.0 {
        return Ok(1.0);
    }

    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();

    if x < (a + 1.0) / (a + b + 2.0) {
        Ok(front * beta_continued_fraction(a, b, x)? / a)
    } else {
        Ok(1.0 - front * beta_continued_fraction(b, a, 1.0 - x)? / b)
    }
}

pub extern "C" fn partitions(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"partitions")?;
    let items = tuple_items(tuple_expr)?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    build_partitions(&items, 0, &mut Vec::new(), &mut out, &mut seen);
    write_partitions(sink, &out)
}

fn expr_is_var(e: Expr) -> Result<bool, EvalError> {
    let raw_var = matches!(
        unsafe { mork_expr::byte_item(*e.ptr) },
        Tag::NewVar | Tag::VarRef(_)
    );
    Ok(raw_var || var_marker_key(e)?.is_some())
}

pub extern "C" fn is_var(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"is_var")?;
    let value = [u8::from(expr_is_var(e)?)];
    sink.write(SourceItem::Symbol(&value))?;
    Ok(())
}

fn expr_is_exp(e: Expr) -> Result<bool, EvalError> {
    let is_compound = matches!(unsafe { mork_expr::byte_item(*e.ptr) }, Tag::Arity(_));
    Ok(is_compound && !expr_is_var(e)?)
}

pub extern "C" fn is_exp(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = match consume_named_expr_1(expr, b"is_exp") {
        Ok(e) => e,
        Err(_) => consume_named_expr_1(expr, b"is-exp")?,
    };
    let value = [u8::from(expr_is_exp(e)?)];
    sink.write(SourceItem::Symbol(&value))?;
    Ok(())
}

pub extern "C" fn vars_to_indices(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"vars_to_indices")?;
    let mut ez = mork_expr::ExprZipper::new(e);
    let mut intro = 0usize;

    loop {
        match ez.item() {
            Ok(Tag::NewVar) => {
                if intro >= 64 {
                    return Err(EvalError::from("can only introduce up to 64 variables"));
                }
                write_var_marker(sink, intro)?;
                intro += 1;
            }
            Ok(Tag::VarRef(i)) => {
                if i == 0 {
                    return Err(EvalError::from(
                        "var reference points outside vars_to_indices argument",
                    ));
                }
                write_var_marker(sink, (i - 1) as usize)?;
            }
            Ok(Tag::Arity(a)) => {
                sink.write(SourceItem::Tag(Tag::Arity(a)))?;
            }
            Ok(Tag::SymbolSize(_)) => unreachable!(),
            Err(symbol) => {
                sink.write(SourceItem::Symbol(symbol))?;
            }
        }

        if !ez.next() {
            break;
        }
    }

    Ok(())
}

pub extern "C" fn indices_to_vars(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"indices_to_vars")?;
    let mut labels = HashMap::new();
    let mut introduced = 0u8;
    write_indices_as_vars(e, sink, &mut labels, &mut introduced)
}

pub extern "C" fn indexed_vars_in_expr(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"indexed_vars_in_expr")?;
    let mut seen = HashSet::new();
    let mut vars = Vec::new();
    collect_indexed_vars(e, &mut seen, &mut vars)?;

    if vars.len() > u8::MAX as usize {
        return Err(EvalError::from("tuple arity exceeds 255"));
    }

    let mut out = Vec::new();
    out.push(item_byte(Tag::Arity(vars.len() as u8)));
    for var in vars {
        out.extend_from_slice(&var);
    }
    write_normalized_expr(sink, out)
}

pub extern "C" fn substitute(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, values_expr) = substitute_args(expr)?;
    let values = tuple_items(values_expr)?;
    let mut state = SubstitutionState::new();
    write_substituted_expr(pattern, sink, &values, &mut state)
}

pub extern "C" fn subsumes(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, target) = subsumes_args(expr, b"subsumes")?;
    write_bool(sink, strictly_subsumes_expr(pattern, target)?)
}

pub extern "C" fn can_unify(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, target) = subsumes_args(expr, b"can_unify")?;
    write_bool(sink, strictly_subsumes_expr(pattern, target)?)
}

pub extern "C" fn subsumed_by(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, other) = subsumes_args(expr, b"subsumed_by")?;
    write_bool(sink, is_subsumed_by_expr(pattern, other)?)
}

pub extern "C" fn not_subsumed(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, other) = subsumes_args(expr, b"not_subsumed")?;
    write_bool(sink, not_subsumed_expr(pattern, other)?)
}

pub extern "C" fn subsumes(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, target) = subsumes_args(expr, b"subsumes")?;
    write_bool(sink, strictly_subsumes_expr(pattern, target)?)
}

pub extern "C" fn can_unify(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, target) = subsumes_args(expr, b"can_unify")?;
    write_bool(sink, strictly_subsumes_expr(pattern, target)?)
}

pub extern "C" fn subsumed_by(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, other) = subsumes_args(expr, b"subsumed_by")?;
    write_bool(sink, is_subsumed_by_expr(pattern, other)?)
}

pub extern "C" fn not_subsumed(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (pattern, other) = subsumes_args(expr, b"not_subsumed")?;
    write_bool(sink, not_subsumed_expr(pattern, other)?)
}

pub extern "C" fn freshen_pattern(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let pattern = consume_named_expr_1(expr, b"freshen-pattern")?;
    write_expr(sink, pattern)
}

pub extern "C" fn factorial(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let items = expr.consume_head_check(b"factorial")?;
    if items != 1 {
        return Err(EvalError::from("factorial takes one argument"));
    }

    let n = expr.consume::<i64>()?;
    let result = factorial_i64(n)?;
    sink.write(SourceItem::Symbol(result.to_be_bytes()[..].into()))?;
    Ok(())
}

pub extern "C" fn falling_factorial(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let items = expr.consume_head_check(b"falling_factorial")?;
    if items != 2 {
        return Err(EvalError::from("falling_factorial takes two arguments"));
    }

    let n = expr.consume::<i64>()?;
    let k = expr.consume::<i64>()?;
    let result = falling_factorial_i64(n, k)?;
    sink.write(SourceItem::Symbol(result.to_be_bytes()[..].into()))?;
    Ok(())
}

pub extern "C" fn beta_cdf_f64(
    expr: *mut ExprSource,
    sink: *mut ExprSink,
) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let items = expr.consume_head_check(b"beta_cdf_f64")?;
    if items != 3 {
        return Err(EvalError::from("beta_cdf_f64 takes three arguments"));
    }

    let alpha = expr.consume::<f64>()?;
    let beta = expr.consume::<f64>()?;
    let x = expr.consume::<f64>()?;
    let result = regularized_beta_cdf(alpha, beta, x)?;
    sink.write(SourceItem::Symbol(result.to_be_bytes()[..].into()))?;
    Ok(())
}

// Counts complete support assignments without retaining their bindings.
// MM2-Helper's installer wires this into MORK's sink dispatcher as
// `(count-fast <output> <count-guard>)`.
pub(crate) struct FastCountSink {
    e: Expr,
    count: usize,
    output: Option<Vec<u8>>,
    guard: Option<Vec<u8>>,
}

impl FastCountSink {
    const HEADER_LEN: usize = 1 + 1 + b"count-fast".len();

    fn write_result<'w, 'a, 'k>(
        wz: &mut WriteZipperTracked<'a, 'k, ()>,
        output: &[u8],
    ) -> bool
    where
        'a: 'w,
        'k: 'w,
    {
        let prefix = wz.root_prefix_path();
        assert!(
            output.starts_with(prefix),
            "count-fast output does not share its requested write prefix"
        );
        wz.move_to_path(&output[prefix.len()..]);
        wz.set_val(()).is_none()
    }

    pub(crate) fn can_skip_render(&self) -> bool {
        self.output.is_some()
    }

    pub(crate) fn skip_rendered_match(&mut self) {
        self.count = self.count.checked_add(1).expect("count-fast overflow");
    }
}

impl Sink for FastCountSink {
    fn new(e: Expr) -> Self {
        Self {
            e,
            count: 0,
            output: None,
            guard: None,
        }
    }

    fn request(&self) -> impl Iterator<Item = WriteResourceRequest> {
        let prefix = unsafe {
            self.e
                .prefix()
                .unwrap_or_else(|_| {
                    let span = self.e.span();
                    std::slice::from_raw_parts(self.e.ptr, span.len() - 1)
                })
                .as_ref()
                .unwrap()
        };
        std::iter::once(WriteResourceRequest::BTM(
            &prefix[Self::HEADER_LEN..],
        ))
    }

    fn sink<'w, 'a, 'k, It: Iterator<Item = WriteResource<'w, 'a, 'k>>>(
        &mut self,
        mut it: It,
        path: &[u8],
    ) where
        'a: 'w,
        'k: 'w,
    {
        let WriteResource::BTM(_) = it.next().unwrap() else {
            unreachable!()
        };
        self.count = self.count.checked_add(1).expect("count-fast overflow");

        if self.output.is_some() {
            return;
        }

        let expression = Expr {
            ptr: path.as_ptr().cast_mut(),
        };
        let mut args = Vec::with_capacity(3);
        ExprEnv::new(0, expression).args(&mut args);
        debug_assert_eq!(args.len(), 3);

        let output = args[1].subsexpr();
        let output = unsafe { output.span().as_ref().unwrap() };
        let guard = args[2].subsexpr();
        let guard = unsafe { guard.span().as_ref().unwrap() };

        self.output = Some(output.to_vec());
        self.guard = Some(guard.to_vec());
    }

    fn finalize<'w, 'a, 'k, It: Iterator<Item = WriteResource<'w, 'a, 'k>>>(
        &mut self,
        mut it: It,
    ) -> bool
    where
        'a: 'w,
        'k: 'w,
    {
        let WriteResource::BTM(wz) = it.next().unwrap() else {
            unreachable!()
        };
        wz.reset();

        let (Some(output), Some(guard)) = (self.output.take(), self.guard.take()) else {
            return false;
        };
        let count = self.count;
        self.count = 0;
        let count_string = count.to_string();

        match mork_expr::byte_item(guard[0]) {
            Tag::NewVar => Self::write_result(wz, &output),
            Tag::VarRef(index) => {
                let mut encoded_count = Vec::with_capacity(1 + count_string.len());
                encoded_count.push(item_byte(Tag::SymbolSize(count_string.len() as _)));
                encoded_count.extend_from_slice(count_string.as_bytes());

                let output_expression = Expr {
                    ptr: output.as_ptr().cast_mut(),
                };
                let count_expression = Expr {
                    ptr: encoded_count.as_mut_ptr(),
                };
                let mut rendered = vec![0_u8; output.len() + encoded_count.len()];
                let mut zipper = ExprZipper::new(Expr {
                    ptr: rendered.as_mut_ptr(),
                });
                output_expression.substitute_one_de_bruijn(index, count_expression, &mut zipper);
                rendered.truncate(zipper.loc);
                Self::write_result(wz, &rendered)
            }
            Tag::SymbolSize(size) => {
                let size = size as usize;
                if guard.len() == size + 1 && &guard[1..] == count_string.as_bytes() {
                    Self::write_result(wz, &output)
                } else {
                    false
                }
            }
            tag => panic!("unsupported count-fast guard: {tag:?}"),
        }
    }
}

pub fn register(scope: &mut EvalScope) {
    scope.add_func("partitions", partitions, FuncType::Pure);
    scope.add_func("is_var", is_var, FuncType::Pure);
    scope.add_func("is_exp", is_exp, FuncType::Pure);
    scope.add_func("is-exp", is_exp, FuncType::Pure);
    scope.add_func("vars_to_indices", vars_to_indices, FuncType::Pure);
    scope.add_func("indices_to_vars", indices_to_vars, FuncType::Pure);
    scope.add_func("indexed_vars_in_expr", indexed_vars_in_expr, FuncType::Pure);
    scope.add_func("substitute", substitute, FuncType::Pure);
    scope.add_func("subsumes", subsumes, FuncType::Pure);
    scope.add_func("can_unify", can_unify, FuncType::Pure);
    scope.add_func("subsumed_by", subsumed_by, FuncType::Pure);
    scope.add_func("not_subsumed", not_subsumed, FuncType::Pure);
    scope.add_func("freshen-pattern", freshen_pattern, FuncType::Pure);
    scope.add_func("factorial", factorial, FuncType::Pure);
    scope.add_func("falling_factorial", falling_factorial, FuncType::Pure);
    scope.add_func("beta_cdf_f64", beta_cdf_f64, FuncType::Pure);
}

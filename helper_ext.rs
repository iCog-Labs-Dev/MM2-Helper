use eval::{EvalScope, FuncType};
use eval_ffi::{EvalError, ExprSink, ExprSource, Tag};
use mork_expr::{item_byte, Expr, ExprEnv, ExprZipper, SourceItem};
use std::collections::HashSet;

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

fn consume_named_expr_2(expr: &mut ExprSource, name: &[u8]) -> Result<(Expr, Expr), EvalError> {
    let items = expr.consume_head_check(name)?;
    if items != 2 {
        return Err(EvalError::from("takes two arguments"));
    }
    Ok((expr.consume::<Expr>()?, expr.consume::<Expr>()?))
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

fn parse_var_marker_index(symbol: &[u8]) -> Result<u8, EvalError> {
    if symbol.is_empty() {
        return Err(EvalError::from("var marker index can not be empty"));
    }

    let mut index = 0u16;
    for &b in symbol {
        if !b.is_ascii_digit() {
            return Err(EvalError::from("var marker index must be decimal digits"));
        }
        index = index * 10 + (b - b'0') as u16;
        if index >= 64 {
            return Err(EvalError::from("var marker index must be less than 64"));
        }
    }

    Ok(index as u8)
}

fn var_marker_index(e: Expr) -> Result<Option<u8>, EvalError> {
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

        let Tag::SymbolSize(index_len) = mork_expr::byte_item(*e.ptr.add(offset)) else {
            return Err(EvalError::from("var marker index must be a symbol"));
        };
        offset += 1;
        let index = std::slice::from_raw_parts(e.ptr.add(offset), index_len as usize);
        Ok(Some(parse_var_marker_index(index)?))
    }
}

fn write_indices_as_vars(e: Expr, sink: &mut ExprSink, introduced: &mut u8) -> Result<(), EvalError> {
    if let Some(index) = var_marker_index(e)? {
        if index == *introduced {
            sink.write(SourceItem::Tag(Tag::NewVar))?;
            *introduced += 1;
        } else if index < *introduced {
            sink.write(SourceItem::Tag(Tag::VarRef(index)))?;
        } else {
            return Err(EvalError::from("var marker index appears before its introduction"));
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
                    let child = Expr { ptr: e.ptr.add(offset) };
                    write_indices_as_vars(child, sink, introduced)?;
                    offset += expr_span(child).len();
                }
            }
        }
    }

    Ok(())
}
fn write_expr(sink: &mut ExprSink, expr: Expr) -> Result<(), EvalError> {
    write_normalized_expr(sink, expr_span(expr).to_vec())
}

fn write_tuple_from_items(sink: &mut ExprSink, items: &[Expr]) -> Result<(), EvalError> {
    let mut out = Vec::new();
    push_tuple_from_items(&mut out, items)?;
    write_normalized_expr(sink, out)
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
        return Err(EvalError::from("falling_factorial expects n >= 0 and k >= 0"));
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

pub extern "C" fn length(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"length")?;
    let n = tuple_items(tuple_expr)?.len() as i64;
    sink.write(SourceItem::Symbol(n.to_be_bytes()[..].into()))?;
    Ok(())
}

pub extern "C" fn car(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"car")?;
    let items = tuple_items(tuple_expr)?;
    if items.is_empty() {
        return Err(EvalError::from("car on empty tuple"));
    }

    write_expr(sink, items[0])
}

pub extern "C" fn cdr(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"cdr")?;
    let items = tuple_items(tuple_expr)?;
    if items.is_empty() {
        return Err(EvalError::from("cdr on empty tuple"));
    }

    write_tuple_from_items(sink, &items[1..])
}

pub extern "C" fn cons(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let (head, tail_tuple) = consume_named_expr_2(expr, b"cons")?;
    let tail_items = tuple_items(tail_tuple)?;

    if tail_items.len() == u8::MAX as usize {
        return Err(EvalError::from("tuple arity exceeds 255"));
    }

    let mut items = Vec::with_capacity(tail_items.len() + 1);
    items.push(head);
    items.extend(tail_items);
    write_tuple_from_items(sink, &items)
}

pub extern "C" fn decons(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"decons")?;
    let items = tuple_items(tuple_expr)?;
    if items.is_empty() {
        return Err(EvalError::from("decons on empty tuple"));
    }

    let mut out = Vec::new();
    out.push(item_byte(Tag::Arity(2)));
    out.extend_from_slice(expr_span(items[0]));
    push_tuple_from_items(&mut out, &items[1..])?;
    write_normalized_expr(sink, out)
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
    let raw_var = matches!(unsafe { mork_expr::byte_item(*e.ptr) }, Tag::NewVar | Tag::VarRef(_));
    Ok(raw_var || var_marker_index(e)?.is_some())
}

pub extern "C" fn is_var(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"is_var")?;
    let value = [u8::from(expr_is_var(e)?)];
    sink.write(SourceItem::Symbol(&value))?;
    Ok(())
}

pub extern "C" fn vars_to_indices(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
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
                    return Err(EvalError::from("var reference points outside vars_to_indices argument"));
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

pub extern "C" fn indices_to_vars(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let e = consume_named_expr_1(expr, b"indices_to_vars")?;
    let mut introduced = 0u8;
    write_indices_as_vars(e, sink, &mut introduced)
}
pub extern "C" fn freshen_pattern(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
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

pub extern "C" fn falling_factorial(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
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

pub fn register(scope: &mut EvalScope) {
    scope.add_func("length", length, FuncType::Pure);
    scope.add_func("car", car, FuncType::Pure);
    scope.add_func("cdr", cdr, FuncType::Pure);
    scope.add_func("cons", cons, FuncType::Pure);
    scope.add_func("decons", decons, FuncType::Pure);
    scope.add_func("partitions", partitions, FuncType::Pure);
    scope.add_func("is_var", is_var, FuncType::Pure);
    scope.add_func("vars_to_indices", vars_to_indices, FuncType::Pure);
    scope.add_func("indices_to_vars", indices_to_vars, FuncType::Pure);
    scope.add_func("freshen-pattern", freshen_pattern, FuncType::Pure);
    scope.add_func("factorial", factorial, FuncType::Pure);
    scope.add_func("falling_factorial", falling_factorial, FuncType::Pure);
}

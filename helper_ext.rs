use eval::{EvalScope, FuncType};
use eval_ffi::{EvalError, ExprSink, ExprSource, Tag};
use mork_expr::{Expr, ExprEnv, SourceItem};
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

fn write_tuple_from_items(sink: &mut ExprSink, items: &[Expr]) -> Result<(), EvalError> {
    sink.write(SourceItem::Tag(Tag::Arity(items.len() as u8)))?;
    for e in items {
        sink.extend_from_slice(expr_span(*e))?;
    }
    Ok(())
}

fn write_tuple_header(sink: &mut ExprSink, len: usize) -> Result<(), EvalError> {
    if len > u8::MAX as usize {
        return Err(EvalError::from("tuple arity exceeds 255"));
    }
    sink.write(SourceItem::Tag(Tag::Arity(len as u8)))?;
    Ok(())
}


fn write_var_marker(sink: &mut ExprSink, index: usize) -> Result<(), EvalError> {
    let index = index.to_string();
    sink.write(SourceItem::Tag(Tag::Arity(2)))?;
    sink.write(SourceItem::Symbol(b"var"))?;
    sink.write(SourceItem::Symbol(index.as_bytes()))?;
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

        let key = Expr { ptr: e.ptr.add(offset) };
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
                    let child = Expr { ptr: e.ptr.add(offset) };
                    write_indices_as_vars(child, sink, labels, introduced)?;
                    offset += expr_span(child).len();
                }
            }
        }
    }

    Ok(())
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
    write_tuple_header(sink, partitions.len())?;
    for partition in partitions {
        write_tuple_header(sink, partition.len())?;
        for block in partition {
            write_tuple_from_items(sink, block)?;
        }
    }
    Ok(())
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

    sink.extend_from_slice(expr_span(items[0]))?;
    Ok(())
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

    sink.write(SourceItem::Tag(Tag::Arity((tail_items.len() + 1) as u8)))?;
    sink.extend_from_slice(expr_span(head))?;
    for e in &tail_items {
        sink.extend_from_slice(expr_span(*e))?;
    }
    Ok(())
}


pub extern "C" fn decons(expr: *mut ExprSource, sink: *mut ExprSink) -> Result<(), EvalError> {
    let expr = unsafe { &mut *expr };
    let sink = unsafe { &mut *sink };

    let tuple_expr = consume_named_expr_1(expr, b"decons")?;
    let items = tuple_items(tuple_expr)?;
    if items.is_empty() {
        return Err(EvalError::from("decons on empty tuple"));
    }

    sink.write(SourceItem::Tag(Tag::Arity(2)))?;
    sink.extend_from_slice(expr_span(items[0]))?;
    write_tuple_from_items(sink, &items[1..])?;
    Ok(())
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
    let mut labels = HashMap::new();
    let mut introduced = 0u8;
    write_indices_as_vars(e, sink, &mut labels, &mut introduced)
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
}

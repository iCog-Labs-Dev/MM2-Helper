use eval::{EvalScope, FuncType};
use eval_ffi::{EvalError, ExprSink, ExprSource, Tag};
use mork_expr::{Expr, ExprEnv, SourceItem};
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

pub fn register(scope: &mut EvalScope) {
    scope.add_func("length", length, FuncType::Pure);
    scope.add_func("car", car, FuncType::Pure);
    scope.add_func("cdr", cdr, FuncType::Pure);
    scope.add_func("cons", cons, FuncType::Pure);
    scope.add_func("decons", decons, FuncType::Pure);
    scope.add_func("partitions", partitions, FuncType::Pure);
}

(subsumes-case example ((eval (var 1) (var 2)) (eval ugly (var 2))))
(subsumes-case repeated-good ((P (var x) (var x)) (P A A)))
(subsumes-case repeated-bad ((P (var x) (var x)) (P A B)))
(subsumes-case concrete-bad ((eval ugly (var 2)) (eval (var 1) (var 2))))
(subsumes-case same-pattern-bad ((eval (var 1) (var 2)) (eval (var 1) (var 2))))
(subsumes-case renamed-pattern-bad ((eval (var 1) (var 2)) (eval (var a) (var b))))
(subsumes-case less-constrained-good ((P (var x) (var y)) (P (var z) (var z))))
(subsumes-case nested-good ((and (P (var x)) (Q (var x))) (and (P A) (Q A))))
(subsumes-case nested-bad ((and (P (var x)) (Q (var x))) (and (P A) (Q B))))
(subsumes-case arity-bad ((P (var x)) (P A B)))

(exec 0
  (, (subsumes-case $id $case))
  (O (pure (subsumes-result $id $out) $out
       (i8_to_string (subsumes (' $case))))))

(can-unify-case alias ((eval (var 1) (var 2)) (eval ugly (var 2))))

(exec 1
  (, (can-unify-case $id $case))
  (O (pure (can-unify-result $id $out) $out
       (i8_to_string (can_unify (' $case))))))

(subsumed-by-case covered ((eval ugly (var 2)) (eval (var 1) (var 2))))
(subsumed-by-case same ((eval (var 1) (var 2)) (eval (var a) (var b))))
(subsumed-by-case uncovered ((eval (var 1) (var 2)) (eval ugly (var 2))))

(exec 2
  (, (subsumed-by-case $id $case))
  (O (pure (subsumed-by-result $id $out) $out
       (i8_to_string (subsumed_by (' $case))))))

(not-subsumed-case covered ((eval ugly (var 2)) (eval (var 1) (var 2))))
(not-subsumed-case similar-good ((eval (var 1) (var 2)) (eval (var a) (var b))))
(not-subsumed-case same-exact-good ((eval (var 1) (var 2)) (eval (var 1) (var 2))))
(not-subsumed-case uncovered ((eval (var 1) (var 2)) (eval ugly (var 2))))
(not-subsumed-case different-concrete ((eval pretty (var 2)) (eval ugly (var 2))))

(exec 3
  (, (not-subsumed-case $id $case))
  (O (pure (not-subsumed-result $id $out) $out
       (i8_to_string (not_subsumed (' $case))))))
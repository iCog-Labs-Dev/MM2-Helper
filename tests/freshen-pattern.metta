;; freshen-pattern copies a pattern into a new local variable scope.
;; Note: current MORK pure evaluation does not preserve repeated textual
;; variable identity once the expression reaches a pure helper.

(pattern repeated (Pair $x $x))
(pattern separate (Pair $x $y))

(exec 0
    (, (pattern $id $pattern))
    (O
        (pure (fresh-pattern $id $fresh) $fresh (freshen-pattern (' $pattern)))))

(Pair A A)
(Pair A B)
(Pair B A)
(Pair B B)

;; Regression test: helper functions must emit variable-safe expressions.
;; Each generated fact is removed immediately. The final result should only
;; contain the original inputs.

(input-cdr (, (A $x) (B $x) (C $x)))
(input-decons (, (A $x) (B $x) (C $x)))
(input-partitions (, (A $x) (B $x) (C $x)))

(exec 1
    (, (input-cdr $pattern))
    (O (pure (made-cdr $tail) $tail (cdr (' $pattern)))))

(exec 1
    (, (input-decons $pattern))
    (O (pure (made-decons $pair) $pair (decons (' $pattern)))))

(exec 1
    (, (input-partitions $pattern))
    (O (pure (made-partitions $parts) $parts (partitions (cdr (' $pattern))))))

(exec 2
    (, (made-cdr $tail))
    (O (- (made-cdr $tail))))

(exec 2
    (, (made-decons $pair))
    (O (- (made-decons $pair))))

(exec 2
    (, (made-partitions $parts))
    (O (- (made-partitions $parts))))

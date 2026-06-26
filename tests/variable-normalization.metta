;; Regression test: MM2-Helper functions must emit variable-safe expressions.
;; Each generated fact is removed immediately. The final result should only
;; contain the original inputs.

(input-partitions ((A $x) (B $x) (C $x)))

(exec 1
    (, (input-partitions $pattern))
    (O (pure (made-partitions $parts) $parts (partitions (' $pattern)))))

(exec 2
    (, (made-partitions $parts))
    (O (- (made-partitions $parts))))

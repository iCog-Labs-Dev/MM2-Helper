;; Factorized counting preserves ordinary conjunction multiplicity and emits
;; an explicit zero result when no assignment exists.

(foo 1)
(foo 2)
(foo 3)
(bar x)
(bar y)
(baz P)
(baz Q)
(baz R)

(exec 0
  (, (foo $x) (bar $y) (baz $z))
  (O (count-factorized (factorized-count-result $count) $count)))

(exec 1
  (, (missing $x))
  (O (count-factorized (factorized-zero-result $count) $count)))

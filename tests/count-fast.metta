;; Smoke test for the MM2-Helper-installed streaming count sink.
;; Expected output includes: (count-fast-result 18)

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
  (O (count-fast (count-fast-result $count) $count)))

; (myexpr (foo $x $z (bar $y $n $l $p)))
(indexed (foo (var (0 arg/0)) (var (0 arg/1)) (bar (var (0 0)) (var (1 0)) (var 00) (var 0))))
(exec 0
  (, (myexpr $e))
  (O (pure (indexed $out) $out
       (vars_to_indices (' $e)))))

(exec 1
  (, (indexed $e))
  (O (pure (roundtrip $out) $out
       (indices_to_vars (' $e)))))
(var-sample $z)
(symbol-sample z)
(index-sample (var 0))

(exec 2
  (, (var-sample $v))
  (O (pure (is-var-result raw $out) $out
       (i8_to_string (is_var (' $v))))))

(exec 3
  (, (symbol-sample $v))
  (O (pure (is-var-result symbol $out) $out
       (i8_to_string (is_var (' $v))))))

(exec 4
  (, (index-sample $v))
  (O (pure (is-var-result indexed $out) $out
       (i8_to_string (is_var (' $v))))))
(exp-sample (foo a b))
(tuple-var-sample (var (0 0)))

(exec 5
  (, (exp-sample $v))
  (O (pure (is-exp-result expression $out) $out
       (i8_to_string (is_exp (' $v))))))

(exec 6
  (, (symbol-sample $v))
  (O (pure (is-exp-result symbol $out) $out
       (i8_to_string (is_exp (' $v))))))

(exec 7
  (, (var-sample $v))
  (O (pure (is-exp-result raw-var $out) $out
       (i8_to_string (is_exp (' $v))))))

(exec 8
  (, (index-sample $v))
  (O (pure (is-exp-result var-marker $out) $out
       (i8_to_string (is_exp (' $v))))))

(exec 9
  (, (tuple-var-sample $v))
  (O (pure (is-exp-result tuple-var-marker $out) $out
       (i8_to_string (is_exp (' $v))))))

(exec 10
  (, (exp-sample $v))
  (O (pure (is-exp-alias-result expression $out) $out
       (i8_to_string (is-exp (' $v))))))

(exec 11
  (, (tuple-var-sample $v))
  (O (pure (is-var-result tuple-marker $out) $out
       (i8_to_string (is_var (' $v))))))

(vars-in-sample (Relation fixed (Wrapper (var 0)) (Pair (var 1) (var 0))))

(exec 12
  (, (vars-in-sample $v))
  (O (pure (indexed-vars-result $out) $out
       (indexed_vars_in_expr (' $v)))))

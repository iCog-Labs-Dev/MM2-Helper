(myexpr (foo $x $x (bar $y)))

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

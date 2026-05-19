(exec 0
    (, (tuple-input $id $t) (head-input $head))
    (O
        (pure (length-result $id $n) $n (i64_to_string (length (' $t))))
        (pure (car-result $id $x) $x (car (' $t)))
        (pure (cdr-result $id $xs) $xs (cdr (' $t)))
        (pure (cons-result $id $xs) $xs (cons $head (' $t)))
        (pure (decons-result $id $pair) $pair (decons (' $t)))
    )
)

(tuple-input simple (a b c))
(tuple-input pattern ((Inheritance abe kebe) (Inheritance abe human) (Inheritance kebe human)))
(head-input h)

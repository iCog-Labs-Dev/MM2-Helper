(exec 0
    (, (input $id $t))
    (O
        (pure
            (partitions-result $id $parts)
            $parts
            (partitions (' $t))
        )
    )
)

(input simple (a b c))
(input pattern ((Inheritance abe kebe) (Inheritance abe human) (Inheritance kebe human)))

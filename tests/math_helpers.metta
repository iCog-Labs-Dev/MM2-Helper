(exec 0
    (, (factorial-input $id $n))
    (O
        (pure
            (factorial-result $id $out)
            $out
            (i64_to_string (factorial (i64_from_string $n)))
        )
    )
)

(exec 0
    (, (binomial-input $id $n $k))
    (O
        (pure
            (binomial-result $id $out)
            $out
            (i64_to_string
                (div_i64
                    (falling_factorial
                        (i64_from_string $n)
                        (i64_from_string $k)
                    )
                    (factorial (i64_from_string $k))
                )
            )
        )
    )
)

(exec 0
    (, (falling-factorial-input $id $n $k))
    (O
        (pure
            (falling-factorial-result $id $out)
            $out
            (i64_to_string
                (falling_factorial
                    (i64_from_string $n)
                    (i64_from_string $k)
                )
            )
        )
    )
)

(exec 0
    (, (beta-cdf-input $id $alpha $beta $x))
    (O
        (pure
            (beta-cdf-result $id $out)
            $out
            (f64_to_string
                (beta_cdf_f64
                    (f64_from_string $alpha)
                    (f64_from_string $beta)
                    (f64_from_string $x)
                )
            )
        )
    )
)

(factorial-input zero 0)
(factorial-input five 5)
(falling-factorial-input twenty-five-three 25 3)
(binomial-input nine-choose-two 9 2)
(binomial-input nine-choose-three 9 3)
(binomial-input twenty-five-choose-three 25 3)
(beta-cdf-input uniform-mid 1.0 1.0 0.5)
(beta-cdf-input beta-two-two-mid 2.0 2.0 0.5)
(beta-cdf-input beta-two-three-mid 2.0 3.0 0.5)

-- One contestant of an edition, over all its matches; $causes is JSON.
INSERT INTO contestants (edition, contestant, prismatic, settings, matches, avg_rank, last_place,
                         avg_value_share_of_winner, avg_idle_cash_share, median_gap_opens, causes)
VALUES ($edition, $contestant, $prismatic, $settings, $matches, $avg_rank, $last_place,
        $avg_value_share_of_winner, $avg_idle_cash_share, $median_gap_opens, $causes);

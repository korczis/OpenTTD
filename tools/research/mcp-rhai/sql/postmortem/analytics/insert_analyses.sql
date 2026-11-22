-- One contestant of one match of an edition; $causes is a comma-separated list of loss causes.
INSERT INTO analyses (edition, match_id, contestant, rank, field, last, value,
                      value_share_of_winner, peak_vehicles, final_vehicles, idle_cash_share,
                      income_per_vehicle, stations_per_vehicle, gap_opens, causes, seed)
VALUES ($edition, $match_id, $contestant, $rank, $field, $last, $value,
        $value_share_of_winner, $peak_vehicles, $final_vehicles, $idle_cash_share,
        $income_per_vehicle, $stations_per_vehicle, $gap_opens, string_split($causes, ','), $seed);

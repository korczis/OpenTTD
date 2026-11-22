-- One settings proposal of an edition for a contestant, with why (JSON).
INSERT INTO proposals (edition, contestant, settings, because)
VALUES ($edition, $contestant, $settings, $because);

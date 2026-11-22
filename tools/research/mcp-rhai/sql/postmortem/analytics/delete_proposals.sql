-- An edition's rows of proposals, removed before the edition is written again. Parameter: $edition.
DELETE FROM proposals WHERE edition = $edition;

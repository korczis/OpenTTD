-- An edition's rows of contestants, removed before the edition is written again. Parameter: $edition.
DELETE FROM contestants WHERE edition = $edition;

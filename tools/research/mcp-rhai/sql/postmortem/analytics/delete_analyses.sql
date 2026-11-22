-- An edition's rows of analyses, removed before the edition is written again. Parameter: $edition.
DELETE FROM analyses WHERE edition = $edition;

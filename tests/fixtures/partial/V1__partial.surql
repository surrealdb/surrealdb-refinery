DEFINE TABLE created_first SCHEMAFULL;
DEFINE FIELD bad ON created_first TYPE int ASSERT $value > 0;
CREATE created_first SET bad = -1;

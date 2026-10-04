DELETE FROM iam.permissions p
USING iam.modules m
WHERE p.module_id = m.id AND m.name IN ('Catalogues Module', 'Customer Module');

DELETE FROM iam.modules WHERE name IN ('Catalogues Module', 'Customer Module');

DELETE FROM iam.role_has_permissions
WHERE permission_id IN (SELECT id FROM iam.permissions WHERE resource = 'Schemas');

DELETE FROM iam.permissions WHERE resource = 'Schemas';

DELETE FROM iam.modules WHERE name = 'Schema Module';

-- An installation the setup wizard (#143) is done with: alice administers
-- through "RH Admins" (common::ADMINS_SID), the stand-in for Kratos' admin
-- account (accounts::ADMIN) directly.
UPDATE instance SET setup_completed_at = now(), setup_step = 6;

INSERT INTO role_assignments (role, principal_sid, principal_kind, principal_name) VALUES
    ('administrator', 'S-1-5-21-1-2-3-1201', 'group', 'RH Admins'),
    ('administrator', 'local:5b0c3cb1-7a0e-4a5e-9d0a-0000000000a2', 'user', 'Admin');

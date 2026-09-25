// What an OpenID Connect provider says about a person, as remotehub keeps
// it (#109): the e-mail address and the name. Kratos uses it when it creates
// an account at a first sign-in, which registration must allow. See
// docs/install.md.
local claims = std.extVar('claims');

{
  identity: {
    traits: {
      email: claims.email,
      [if 'name' in claims then 'name' else null]: claims.name,
    },
  },
}

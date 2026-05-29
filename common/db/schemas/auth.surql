# Defines authentication scope and access rules.
# This mirrors the logic previously in SurrealDbClient::setup_auth

DEFINE ACCESS IF NOT EXISTS account ON DATABASE TYPE RECORD
    SIGNUP ( CREATE user SET email = $email, password = crypto::argon2::generate($password), anonymous = false, user_id = $user_id) # Ensure user_id is provided if needed
    SIGNIN ( SELECT * FROM user WHERE email = $email AND crypto::argon2::compare(password, $password) );

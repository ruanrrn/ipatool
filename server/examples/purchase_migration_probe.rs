fn main() {
    let path = std::env::args().nth(1).expect("database copy path");
    let _db = ipa_webtool_services::Database::new(&path).expect("migration failed");
    println!("Database::new migrations completed");
}

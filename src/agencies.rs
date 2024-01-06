pub fn agency_readable(agency: &str) -> &str {
    match agency {
        "SF" => "Muni",
        x => x,
    }
}

pub fn classify_email(
    subject: &str,
    snippet: &str,
    from_address: &str,
    company_name: &str,
    job_title: &str,
) -> Option<&'static str> {
    let haystack = format!("{subject} {snippet} {from_address}").to_lowercase();
    let company = company_name.to_lowercase();
    let title = job_title.to_lowercase();
    let has_company = company.len() > 2 && haystack.contains(&company);
    let has_title = title.len() > 2 && haystack.contains(&title);
    let application_signals = [
        "application",
        "interview",
        "thank you for applying",
        "next steps",
        "offer",
        "unfortunately",
        "status update",
    ];
    let has_signal = application_signals.iter().any(|s| haystack.contains(s));

    if has_company && has_title && has_signal {
        Some("high")
    } else if (has_company || has_title) && has_signal {
        Some("medium")
    } else if has_company || has_title {
        Some("low")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_confidence() {
        assert_eq!(
            classify_email(
                "Acme interview for Senior Designer",
                "Thanks for your application",
                "recruiting@acme.com",
                "Acme",
                "Senior Designer",
            ),
            Some("high")
        );
    }

    #[test]
    fn low_confidence() {
        assert_eq!(
            classify_email(
                "Acme monthly newsletter",
                "Product updates this month",
                "hello@acme.com",
                "Acme",
                "Senior Designer",
            ),
            Some("low")
        );
    }
}

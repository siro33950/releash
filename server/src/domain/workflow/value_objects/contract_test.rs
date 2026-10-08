mod contract_tests {
    use super::super::*;

    #[test]
    fn test_contract_type_emptyを拒否する() {
        assert!(ContractType::new("spec-directory").is_ok());
        assert!(ContractType::new(" ").is_err());
    }
}

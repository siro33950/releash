#[test]
fn test_rpc_scopeは空と未指定と重複を拒否する() {
    for scopes in [&[][..], &[0], &[1, 0], &[1, 1], &[2, 2], &[3]] {
        assert!(!super::valid_scopes(scopes), "{scopes:?}");
    }
    for scopes in [&[1][..], &[2], &[1, 2], &[2, 1]] {
        assert!(super::valid_scopes(scopes), "{scopes:?}");
    }
}

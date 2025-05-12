WITH systemderiv AS (
        SELECT id FROM ValidPaths
        WHERE path = '/nix/store/jz9m7fs5m87zrcy50kxgzay2ksah2n2r-nixos-system-flocke-25.05.20250508.dda3dcd'
    ),
    systempath AS (
        SELECT reference as id FROM systemderiv sd
        JOIN Refs ON sd.id = referrer
        JOIN ValidPaths vp ON reference = vp.id
        WHERE (vp.path LIKE '%-system-path')
    ),
    pkgs AS (
        SELECT reference as id FROM Refs
        JOIN systempath ON referrer = id
    )
SELECT pkgs.id, path FROM pkgs
JOIN ValidPaths vp ON vp.id = pkgs.id;

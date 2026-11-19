equalizer-advanced = I avancuar
equalizer-simple = I thjeshtë
equalizer-sound = Tingulli
equalizer-profile-default = Parazgjedhur
equalizer-profile-new = Profili { $number }
equalizer-profile-pasted = Profil i ngjitur
equalizer-profile-name-hint = Emri i profilit
equalizer-save-as = Ruaj si profil
equalizer-rename = Riemërto profilin
equalizer-delete = Fshi profilin
equalizer-bass = Basi
equalizer-treble = Treble
equalizer-tone-reset = Rivendos në 0 dB
equalizer-preamp = Paraamplifikimi
equalizer-preamp-auto = Automatik
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = okt
equalizer-no-gain = —
equalizer-filters = Filtrat
equalizer-no-filters = Ende asnjë filtër. Shto një, zgjidh një paracaktim më lart, ose importo një profil.
equalizer-add-filter = Shto filtër
equalizer-remove-filter = Hiq filtrin
equalizer-kind-peak = Kulm
equalizer-kind-low-shelf = Raft i ulët
equalizer-kind-high-shelf = Raft i lartë
equalizer-kind-low-pass = Kalim i ulët
equalizer-kind-high-pass = Kalim i lartë
equalizer-kind-notch = Prerës
equalizer-kind-band-pass = Kalim brezi
equalizer-kind-all-pass = Kalim i plotë
equalizer-transfer = Importo dhe eksporto
equalizer-import = Importo
equalizer-export = Eksporto
equalizer-import-file = Importo skedar…
equalizer-export-file = Eksporto skedar…
equalizer-imported = U importua { $name }
equalizer-imported-skipped = U importua { $name } pa { $count ->
        [one] një rresht
       *[other] { $count } rreshta
    }
equalizer-import-nothing = Nuk u gjetën cilësime ekualizuesi për t'u importuar
equalizer-import-too-many = Nuk u importua: { $count ->
        [one] një filtër është aktiv
       *[other] { $count } filtra janë aktivë
    }, më shumë se sa mban një profil
equalizer-import-too-large = Nuk u importua: teksti kalon 1 MB, shumë i madh për një profil ekualizuesi
equalizer-filter-limit =Një profil mban deri në { $max } filtra.
equalizer-paste-empty = Kujtesa e fragmenteve nuk ka tekst
equalizer-copied = Profili u kopjua si tekst Equalizer APO
equalizer-exported = Profili u eksportua
equalizer-file-unreadable = Skedari nuk mund të lexohej
equalizer-file-unwritable = Skedari nuk mund të shkruhej
equalizer-value-refused = U mbajt vlera e mëparshme
equalizer-skipped-line = Rreshti { $line }: { $reason }
equalizer-skipped-more = dhe { $count ->
        [one] një tjetër
       *[other] { $count } të tjerë
    }
equalizer-skip-unknown-type = lloj i panjohur filtri { $name }
equalizer-skip-unreadable = nuk është filtër që Sonora mund ta lexojë
equalizer-skip-no-frequency = pa frekuencë
equalizer-skip-no-gain = pa përforcim
equalizer-skip-no-width = pa Q ose gjerësi brezi
equalizer-skip-expression = një vlerë e shkruar si shprehje
equalizer-skip-no-preamp = paraamplifikimi nuk është numër
equalizer-skip-unsupported = { $name } nuk mbështetet
equalizer-rules = Rregullat e pajisjeve
equalizer-rule-other = Çdo pajisje tjetër
equalizer-rule-playing = Po luan tani
equalizer-rule-replaces = Zëvendëson rregullin e saj
equalizer-rule-choose-device = Zgjidh një pajisje
equalizer-rule-choose-sound = Zgjidh një tingull
equalizer-rule-add = Shto rregull
equalizer-rule-remove = Hiq rregullin
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Frekuenca duhet të jetë midis { $low } dhe { $high } Hz
equalizer-invalid-gain = Përforcimi duhet të jetë midis { $low } dhe { $high } dB
equalizer-invalid-q = Q duhet të jetë midis { $low } dhe { $high }
equalizer-invalid-octaves = Gjerësia e brezit duhet të jetë midis { $low } dhe { $high } oktava
equalizer-invalid-shelf-octaves = Një raft merr Q, jo gjerësi brezi
equalizer-invalid-preamp = Paraamplifikimi duhet të jetë midis { $low } dhe { $high } dB
equalizer-overload = Prerje me { $db } dB
equalizer-overload-slight = Prerje nën 0,1 dB
equalizer-lower-preamp = Ul paraamplifikimin
equalizer-level = Niveli
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect nga Sonora
equalizer-status-server = Serveri i zërit pastaj e konverton në { $khz } kHz
equalizer-status-altered = Jo bit-perfect për shkak të:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volumi
equalizer-status-normalized = Normalizuar
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } kan.
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } kan.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB

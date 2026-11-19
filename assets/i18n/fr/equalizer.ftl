equalizer-advanced = Avancé
equalizer-simple = Simple
equalizer-sound = Son
equalizer-profile-default = Par défaut
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Profil collé
equalizer-profile-name-hint = Nom du profil
equalizer-save-as = Enregistrer comme profil
equalizer-rename = Renommer le profil
equalizer-delete = Supprimer le profil
equalizer-bass = Graves
equalizer-treble = Aigus
equalizer-tone-reset = Remettre à 0 dB
equalizer-preamp = Préampli
equalizer-preamp-auto = Automatique
equalizer-preamp-manual = Manuel
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct.
equalizer-no-gain = —
equalizer-filters = Filtres
equalizer-no-filters = Aucun filtre pour l'instant. Ajoutez-en un, choisissez un préréglage ci-dessus ou importez un profil.
equalizer-add-filter = Ajouter un filtre
equalizer-remove-filter = Retirer le filtre
equalizer-kind-peak = Cloche
equalizer-kind-low-shelf = Plateau grave
equalizer-kind-high-shelf = Plateau aigu
equalizer-kind-low-pass = Passe-bas
equalizer-kind-high-pass = Passe-haut
equalizer-kind-notch = Réjecteur
equalizer-kind-band-pass = Passe-bande
equalizer-kind-all-pass = Passe-tout
equalizer-transfer = Importer et exporter
equalizer-import = Importer
equalizer-export = Exporter
equalizer-import-file = Importer un fichier…
equalizer-export-file = Exporter un fichier…
equalizer-imported = { $name } importé
equalizer-imported-skipped = { $name } importé sans { $count ->
        [one] une ligne
       *[other] { $count } lignes
    }
equalizer-import-nothing = Aucun réglage d'égaliseur à importer
equalizer-import-too-many = Non importé : { $count ->
        [one] un filtre est actif
       *[other] { $count } filtres sont actifs
    }, plus qu'un profil ne peut en contenir
equalizer-import-too-large = Non importé : le texte dépasse 1 Mo, trop pour un profil d'égaliseur
equalizer-filter-limit =Un profil contient au plus { $max } filtres.
equalizer-paste-empty = Le presse-papiers ne contient pas de texte
equalizer-copied = Profil copié au format Equalizer APO
equalizer-exported = Profil exporté
equalizer-file-unreadable = Impossible de lire le fichier
equalizer-file-unwritable = Impossible d'écrire le fichier
equalizer-value-refused = Valeur précédente conservée
equalizer-skipped-line = Ligne { $line } : { $reason }
equalizer-skipped-more = et { $count ->
        [one] une de plus
       *[other] { $count } de plus
    }
equalizer-skip-unknown-type = type de filtre inconnu { $name }
equalizer-skip-unreadable = pas un filtre que Sonora sait lire
equalizer-skip-no-frequency = pas de fréquence
equalizer-skip-no-gain = pas de gain
equalizer-skip-no-width = ni Q ni largeur de bande
equalizer-skip-expression = une valeur écrite sous forme d'expression
equalizer-skip-no-preamp = le préampli n'est pas un nombre
equalizer-skip-unsupported = { $name } n'est pas pris en charge
equalizer-rules = Règles par périphérique
equalizer-rule-other = Tout autre périphérique
equalizer-rule-playing = En lecture
equalizer-rule-replaces = Remplace sa règle
equalizer-rule-choose-device = Choisir un périphérique
equalizer-rule-choose-sound = Choisir un son
equalizer-rule-add = Ajouter une règle
equalizer-rule-remove = Retirer la règle
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = La fréquence doit être comprise entre { $low } et { $high } Hz
equalizer-invalid-gain = Le gain doit être compris entre { $low } et { $high } dB
equalizer-invalid-q = Le Q doit être compris entre { $low } et { $high }
equalizer-invalid-octaves = La largeur de bande doit être comprise entre { $low } et { $high } octaves
equalizer-invalid-shelf-octaves = Un plateau prend un Q, pas une largeur de bande
equalizer-invalid-preamp = Le préampli doit être compris entre { $low } et { $high } dB
equalizer-overload = Écrêtage de { $db } dB
equalizer-overload-slight = Écrêtage inférieur à 0,1 dB
equalizer-lower-preamp = Baisser le préampli
equalizer-level = Niveau
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect depuis Sonora
equalizer-status-server = Le serveur de son le convertit ensuite en { $khz } kHz
equalizer-status-altered = Pas bit-perfect à cause de :
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Normalisé
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bits
equalizer-status-float = Flottant → { $to } bits
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stéréo
   *[other] { $from } canaux
} → { $to ->
    [1] mono
    [2] stéréo
   *[other] { $to } canaux
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB

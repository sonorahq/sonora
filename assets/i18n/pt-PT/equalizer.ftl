equalizer-advanced = Avançado
equalizer-simple = Simples
equalizer-sound = Som
equalizer-profile-default = Predefinido
equalizer-profile-new = Perfil { $number }
equalizer-profile-pasted = Perfil colado
equalizer-profile-name-hint = Nome do perfil
equalizer-save-as = Guardar como perfil
equalizer-rename = Renomear perfil
equalizer-delete = Eliminar perfil
equalizer-bass = Graves
equalizer-treble = Agudos
equalizer-tone-reset = Repor para 0 dB
equalizer-preamp = Pré-amp
equalizer-preamp-auto = Automático
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oit
equalizer-no-gain = —
equalizer-filters = Filtros
equalizer-no-filters = Nenhum filtro ainda. Adicione um, escolha uma predefinição acima ou importe um perfil.
equalizer-add-filter = Adicionar filtro
equalizer-remove-filter = Remover filtro
equalizer-kind-peak = Pico
equalizer-kind-low-shelf = Shelf de graves
equalizer-kind-high-shelf = Shelf de agudos
equalizer-kind-low-pass = Passa-baixas
equalizer-kind-high-pass = Passa-altas
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Passa-faixa
equalizer-kind-all-pass = Passa-tudo
equalizer-transfer = Importar e exportar
equalizer-import = Importar
equalizer-export = Exportar
equalizer-import-file = Importar ficheiro…
equalizer-export-file = Exportar ficheiro…
equalizer-imported = { $name } importado
equalizer-imported-skipped = { $name } importado sem { $count ->
        [one] uma linha
       *[other] { $count } linhas
    }
equalizer-import-nothing = Nenhuma definição de equalizador encontrada para importar
equalizer-import-too-many = Não importado: { $count ->
        [one] um filtro está ativo
       *[other] { $count } filtros estão ativos
    }, mais do que um perfil comporta
equalizer-import-too-large = Não importado: o texto ultrapassa 1 MB, demasiado grande para um perfil de equalizador
equalizer-filter-limit = Um perfil comporta até { $max } filtros.
equalizer-paste-empty = A área de transferência não tem texto
equalizer-copied = Perfil copiado como texto do Equalizer APO
equalizer-exported = Perfil exportado
equalizer-file-unreadable = Não foi possível ler o ficheiro
equalizer-file-unwritable = Não foi possível gravar o ficheiro
equalizer-value-refused = O valor anterior foi mantido
equalizer-skipped-line = Linha { $line }: { $reason }
equalizer-skipped-more = e { $count ->
        [one] mais uma
       *[other] mais { $count }
    }
equalizer-skip-unknown-type = tipo de filtro desconhecido { $name }
equalizer-skip-unreadable = não é um filtro que o Sonora consegue ler
equalizer-skip-no-frequency = sem frequência
equalizer-skip-no-gain = sem ganho
equalizer-skip-no-width = sem Q ou largura de banda
equalizer-skip-expression = um valor escrito como expressão
equalizer-skip-no-preamp = o pré-amp não é um número
equalizer-skip-unsupported = { $name } não é suportado
equalizer-rules = Regras por dispositivo
equalizer-rule-other = Qualquer outro dispositivo
equalizer-rule-playing = Em reprodução
equalizer-rule-replaces = Substitui a regra existente
equalizer-rule-choose-device = Escolha um dispositivo
equalizer-rule-choose-sound = Escolha um som
equalizer-rule-add = Adicionar regra
equalizer-rule-remove = Remover regra
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = A frequência deve estar entre { $low } e { $high } Hz
equalizer-invalid-gain = O ganho deve estar entre { $low } e { $high } dB
equalizer-invalid-q = O Q deve estar entre { $low } e { $high }
equalizer-invalid-octaves = A largura de banda deve estar entre { $low } e { $high } oitavas
equalizer-invalid-shelf-octaves = Um shelf usa Q, não largura de banda
equalizer-invalid-preamp = O pré-amp deve estar entre { $low } e { $high } dB
equalizer-overload = Clipping de { $db } dB
equalizer-overload-slight = Clipping abaixo de 0,1 dB
equalizer-lower-preamp = Reduzir pré-amp
equalizer-level = Nível
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect a partir do Sonora
equalizer-status-server = Depois, o servidor de som converte para { $khz } kHz
equalizer-status-altered = Não é bit-perfect por causa de:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Normalizado
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bits
equalizer-status-float = Float → { $to } bits
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Estéreo
   *[other] { $from } canais
} → { $to ->
    [1] mono
    [2] estéreo
   *[other] { $to } canais
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB

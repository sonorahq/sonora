# track menu
menu-new-playlist = Nova playlist
menu-import-playlist = Importar playlist…
menu-no-playlists = Nenhuma playlist
menu-search-playlists = Pesquisar playlist
menu-save-library = Biblioteca
menu-add-to-library = Adicionar às Favoritas
menu-remove-from-library = Remover das Favoritas
menu-remove-tracks-from-playlist = { $count ->
    [one] Remover { $count } faixa da playlist
   *[other] Remover { $count } faixas da playlist
}
menu-remove-tracks-from-history = { $count ->
    [one] Remover { $count } faixa do histórico
   *[other] Remover { $count } faixas do histórico
}
menu-download = Baixar
menu-remove-download = Remover download
menu-downloading = Baixando. Clique para parar
menu-song-radio = Ir para a rádio da música
menu-go-to-album = Ir para o álbum
menu-go-to-artist = Ir para o artista
menu-go-to-playlist = Ir para a playlist
menu-view-details = Ver detalhes
menu-copy-link = Copiar link
menu-select-all = Selecionar tudo
menu-remove-from-queue = Remover da fila
menu-export-playlist = Exportar playlist…
menu-add-playlist-to-library = Adicionar à Biblioteca
menu-remove-playlist-from-library = Remover da Biblioteca
menu-make-playlist-public = Tornar pública
menu-make-playlist-private = Tornar privada
menu-play-artist = Tocar artista

# playlist editor
menu-copy-link-none = Isto não tem link para compartilhar
menu-copy-link-local = Artistas e playlists locais não têm link
menu-play-next = { $count ->
    [1] Tocar a seguir
    [one] Tocar { $count } faixa a seguir
   *[other] Tocar { $count } faixas a seguir
}
menu-add-to-queue = { $count ->
    [1] Adicionar à fila
    [one] Adicionar { $count } faixa à fila
   *[other] Adicionar { $count } faixas à fila
}
menu-play-last = { $count ->
    [1] Tocar por último
    [one] Tocar { $count } faixa por último
   *[other] Tocar { $count } faixas por último
}
menu-delete-track-files = { $count ->
    [1] Excluir arquivo da faixa
   *[other] Excluir { $count } arquivos de faixa
}

# playlist editor
menu-add-to-playlist = { $count ->
    [1] Adicionar à playlist
    [one] Adicionar { $count } faixa à playlist
   *[other] Adicionar { $count } faixas à playlist
}
smart-rules-create-title = Nova playlist inteligente
smart-rules-edit-title = Regras inteligentes
smart-rules-loading = Carregando as regras do servidor…
smart-rules-saving = Salvando…
smart-rules-failed-title = Não foi possível carregar as regras
smart-rules-match-all = Atender a cada uma
smart-rules-match-any = Atender a qualquer uma
smart-rules-match-toggle = Atender a todas as regras ou a qualquer uma delas
smart-rules-of-following = das regras a seguir
smart-rules-add-condition = Adicionar condição
smart-rules-add-group = Adicionar grupo
smart-rules-remove-group = Remover este grupo
smart-rules-remove-condition = Remover esta regra
smart-rules-empty-group = Nenhuma regra neste grupo ainda
smart-rules-field = Campo
smart-rules-operator = Operador
smart-rules-value-placeholder = Valor
smart-rules-range-and = e
smart-rules-true = Sim
smart-rules-false = Não
smart-rules-unknown-note = Mantida como foi enviada pelo servidor
smart-rules-synced-title = Esta playlist é sincronizada a partir de um arquivo
smart-rules-synced-message = A próxima varredura pode sobrescrever as alterações feitas aqui.
smart-rules-sort-label = Ordenar por
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = Sem ordem definida
smart-rules-order-asc = Crescente
smart-rules-order-desc = Decrescente
smart-rules-order-toggle = Sentido da ordenação
smart-rules-limit-label = Limite
smart-rules-limit-placeholder = Todos os resultados
smart-rules-problem-empty-group = O grupo não contém regras
smart-rules-problem-missing-field = A condição não especifica um campo
smart-rules-problem-range = Um intervalo precisa de um limite inferior e um superior
smart-rules-problem-days = A quantidade de dias deve ser um número
smart-rules-problem-playlist = Uma regra de playlist precisa de uma playlist
smart-rules-problem-truth = A regra precisa de um valor verdadeiro ou falso
smart-rules-problem-compared = O valor não é válido para esta regra
smart-rules-problem-boolean = Um campo booleano precisa de um valor verdadeiro ou falso
smart-rules-problem-bad-order = A ordem deve ser asc ou desc
playlist-name-placeholder = Nome da playlist
playlist-create-title = Criar playlist
playlist-create-shelf = Salvar em
playlist-edit-title = Editar playlist
playlist-cover-choose = Escolher imagem
playlist-cover-remove = Remover capa
playlist-cover-unopenable = O Sonora não consegue abrir este arquivo
playlist-cover-too-large = Esta imagem tem mais de 40 MB
playlist-cover-not-a-picture = Este arquivo não é uma imagem JPEG, PNG ou WebP
playlist-delete-title = Excluir playlist
playlist-again-title = Adicionar novamente?
playlist-again-confirm = Esta faixa já está em "{ $name }". Adicionar outra cópia?
playlist-again-add = Adicionar novamente

# confirm

# confirm
cover-view = Ver capa
cover-zoom-in = Ampliar
cover-zoom-out = Reduzir
cover-zoom-fit = Ajustar à janela
confirm-delete-track-files-title = Excluir arquivos das faixas?
confirm-delete-track-files = Os arquivos serão excluídos permanentemente do disco. Isto não pode ser desfeito.
confirm-uninstall-widevine-title = Desinstalar o módulo Widevine?
confirm-uninstall-widevine = O Sonora exclui a cópia que baixou do Google. As faixas do Apple Music param de tocar até que ele seja baixado novamente.
confirm-remove-soloist-title = Remover o Soloist?
confirm-remove-soloist = O Sonora exclui o programa Soloist, sua chave de API e os dados de login dele. O Spotify volta a usar o reprodutor integrado.
confirm-remove-downloads-title = Remover todos os downloads?
confirm-remove-downloads = Todas as músicas que você baixou são excluídas deste computador, de todos os serviços. Elas voltam a ser transmitidas até você baixá-las de novo.
confirm-delete-named = Excluir "{ $name }"? Isto não pode ser desfeito.

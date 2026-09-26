# Regras que acompanham este plugin para dentro do app que o usa.
#
# O `build.gradle.kts` já as declarava em `consumerProguardFiles`, mas o arquivo
# não existia — só passou a fazer falta quando o App Remote entrou.
#
# ------------------------------------------------------------------
# App Remote do Spotify
# ------------------------------------------------------------------
#
# O R8 derruba o build de release com:
#
#   Missing class com.spotify.base.annotations.NotNull
#     (referenced from: SpotifyServiceBinder.bindService(...))
#
# É uma anotação de tempo de COMPILAÇÃO que o AAR referencia e não embute —
# ela não existe em tempo de execução e nada a procura ali. O próprio
# `proguard.txt` do AAR silencia várias classes ausentes (`protocol.types.*`,
# `com.fasterxml.jackson.*`) e simplesmente esqueceu desta.
#
# `-dontwarn` e não `-keep`: não há o que manter, a classe não vem no pacote.
-dontwarn com.spotify.base.annotations.**

# O App Remote conversa com o app do Spotify serializando por Gson, e Gson
# trabalha por reflexão: um campo renomeado pelo R8 vira um campo que o outro
# lado não reconhece, e a falha apareceria só no carro, como "não tocou".
#
# O AAR já manda manter quem implementa `protocol.types.Item`; isto cobre o
# resto do pacote de tipos, que é pequeno e não vale o risco.
-keep class com.spotify.protocol.types.** { *; }

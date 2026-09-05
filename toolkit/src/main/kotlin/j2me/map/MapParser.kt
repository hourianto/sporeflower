package j2me.map

import com.github.javaparser.ParseProblemException
import com.github.javaparser.Problem
import com.github.javaparser.ast.CompilationUnit
import com.github.javaparser.ast.nodeTypes.NodeWithRange
import com.github.javaparser.ast.body.ClassOrInterfaceDeclaration
import com.github.javaparser.ast.body.ConstructorDeclaration
import com.github.javaparser.ast.body.FieldDeclaration
import com.github.javaparser.ast.body.MethodDeclaration
import com.github.javaparser.ast.body.Parameter
import com.github.javaparser.ast.expr.AnnotationExpr
import com.github.javaparser.ast.expr.CharLiteralExpr
import com.github.javaparser.ast.expr.ClassExpr
import com.github.javaparser.ast.expr.Expression
import com.github.javaparser.ast.expr.IntegerLiteralExpr
import com.github.javaparser.ast.expr.LongLiteralExpr
import com.github.javaparser.ast.expr.NormalAnnotationExpr
import com.github.javaparser.ast.expr.SingleMemberAnnotationExpr
import com.github.javaparser.ast.expr.UnaryExpr
import j2me.common.isValidIdentifier
import j2me.common.isValidReadableClassName
import j2me.common.parseCompilationUnit
import j2me.common.readableClassToInternal
import j2me.common.TypeDescriptorResolution
import j2me.common.validateBytecodeClassName
import j2me.common.wasClassOwnerRegex
import j2me.common.wasMemberRegex
import j2me.common.JavaSourceContext
import j2me.common.javaSourceContext
import j2me.common.typeDescriptorResolution
import j2me.common.typeNodeToDescriptor
import j2me.model.CanonicalMap
import j2me.model.ClassSymbols
import j2me.model.FieldSig
import j2me.model.MappingOrigin
import j2me.model.MethodSig
import j2me.model.ProjectMappings
import j2me.model.SemanticDomain
import j2me.model.SemanticDomainKind
import j2me.model.SemanticTarget
import j2me.model.SyntheticSemanticValue
import j2me.model.SemanticStringValue
import j2me.validation.MappingValidationException
import j2me.validation.ValidationIssue
import org.objectweb.asm.Type
import java.nio.file.Path
import kotlin.io.path.invariantSeparatorsPathString
import kotlin.io.path.exists
import kotlin.io.path.listDirectoryEntries
import kotlin.io.path.name
import kotlin.io.path.readText
import kotlin.io.path.relativeTo

private fun registerClassMapping(
    readableName: String,
    obfOwnerRaw: String,
    classes: MutableMap<String, String>,
    readableToObf: MutableMap<String, String>,
    ignoredClasses: Set<String>,
    errors: MutableList<String>,
    source: String,
) {
    val obfOwner = readableClassToInternal(obfOwnerRaw)
    val targetInternal = readableClassToInternal(readableName)

    if (!isValidReadableClassName(readableName)) {
        errors += "$source: invalid readable class name: $readableName"
        return
    }
    if (!validateBytecodeClassName(obfOwner)) {
        errors += "$source: invalid obfuscated owner: $obfOwnerRaw"
        return
    }

    if (obfOwner in ignoredClasses) {
        errors += "$source: class owner is marked @AlreadyMapped and cannot also be remapped: $obfOwnerRaw"
        return
    }

    val existingReadable = readableToObf[readableName]
    if (existingReadable != null && existingReadable != obfOwner) {
        errors += "$source: duplicate readable class name with different owner: $readableName"
        return
    }

    val existingClass = classes[obfOwner]
    if (existingClass != null && existingClass != targetInternal) {
        errors += "$source: duplicate obfuscated owner with different target: $obfOwnerRaw"
        return
    }

    readableToObf[readableName] = obfOwner
    classes[obfOwner] = targetInternal
}

private fun registerIgnoredClassMapping(
    readableName: String,
    knownProjectClasses: Set<String>,
    classes: Map<String, String>,
    readableToObf: MutableMap<String, String>,
    ignoredClasses: MutableSet<String>,
    errors: MutableList<String>,
    source: String,
) {
    val owner = readableClassToInternal(readableName)

    if (!isValidReadableClassName(readableName)) {
        errors += "$source: invalid readable class name: $readableName"
        return
    }
    if (owner !in knownProjectClasses) {
        errors += "$source: @AlreadyMapped class not found in jar: $owner"
        return
    }

    val existingReadable = readableToObf[readableName]
    if (existingReadable != null && existingReadable != owner) {
        errors += "$source: duplicate readable class name with different owner: $readableName"
        return
    }

    val existingClass = classes[owner]
    if (existingClass != null) {
        errors += "$source: class owner is already remapped and cannot be marked @AlreadyMapped: $owner -> $existingClass"
        return
    }

    readableToObf[readableName] = owner
    ignoredClasses += owner
}

private fun addDefaultPackageAliases(
    readableToObf: MutableMap<String, String>,
    errors: MutableList<String>,
    source: String,
) {
    val entries = readableToObf.toMap()
    for ((readableName, obfOwner) in entries) {
        val alias = when {
            '.' !in readableName -> "defpackage.$readableName"
            readableName.startsWith("defpackage.") && readableName.count { it == '.' } == 1 -> readableName.substringAfter('.')
            else -> null
        }
        if (alias == null) {
            continue
        }

        val existing = readableToObf[alias]
        when {
            existing == null -> readableToObf[alias] = obfOwner
            existing != obfOwner -> {
                errors += "$source: default-package alias conflict: '$readableName' maps to $obfOwner, but '$alias' maps to $existing"
            }
        }
    }
}

private fun rangeSlice(lines: List<String>, beginLine: Int, endLine: Int): String {
    val startIdx = (beginLine - 1).coerceAtLeast(0)
    val endExclusive = endLine.coerceAtMost(lines.size)
    if (startIdx >= endExclusive || startIdx >= lines.size) {
        return ""
    }
    return lines.subList(startIdx, endExclusive).joinToString("\n")
}

private fun extractWasMemberComment(sourceSnippet: String): String? = wasMemberRegex.find(sourceSnippet)?.groupValues?.get(1)

private fun extractWasClassOwnerComment(sourceSnippet: String): String? = wasClassOwnerRegex.find(sourceSnippet)?.groupValues?.get(1)

private val ignoredClassAnnotations = setOf("AlreadyMapped")
private val semanticDomainAnnotations = mapOf(
    "ValueDomain" to SemanticDomainKind.VALUE,
    "FlagDomain" to SemanticDomainKind.FLAGS,
    "SlotDomain" to SemanticDomainKind.SLOTS,
    "PackedDomain" to SemanticDomainKind.PACKED,
    "NumericDomain" to SemanticDomainKind.NUMERIC,
    "StringDomain" to SemanticDomainKind.STRING,
)

private fun annotationNamed(annotations: Iterable<AnnotationExpr>, name: String): AnnotationExpr? =
    annotations.firstOrNull { it.nameAsString.substringAfterLast('.') == name }

private fun semanticDomainKind(classDecl: ClassOrInterfaceDeclaration): SemanticDomainKind? =
    classDecl.annotations.firstNotNullOfOrNull { semanticDomainAnnotations[it.nameAsString.substringAfterLast('.')] }

internal fun annotationValue(annotation: AnnotationExpr, name: String = "value"): Expression? = when (annotation) {
    is SingleMemberAnnotationExpr -> if (name == "value") annotation.memberValue else null
    is NormalAnnotationExpr -> annotation.pairs.firstOrNull { it.nameAsString == name }?.value
    else -> null
}

internal fun annotationClassName(annotation: AnnotationExpr, name: String = "value"): String {
    val expr = annotationValue(annotation, name)
        ?: throw IllegalArgumentException("@$name annotation value is missing")
    return (expr as? ClassExpr)?.typeAsString
        ?: throw IllegalArgumentException("annotation value must be a class literal, got: $expr")
}

internal fun parseIntegralConstant(expr: Expression): Long = when (expr) {
    is CharLiteralExpr -> expr.asChar().code.toLong()
    is IntegerLiteralExpr -> expr.asNumber().toLong()
    is LongLiteralExpr -> expr.asNumber().toLong()
    is UnaryExpr -> when (expr.operator) {
        UnaryExpr.Operator.MINUS -> -parseIntegralConstant(expr.expression)
        UnaryExpr.Operator.PLUS -> parseIntegralConstant(expr.expression)
        else -> throw IllegalArgumentException("constant must be an integral literal, got: $expr")
    }
    else -> throw IllegalArgumentException("constant must be an integral literal, got: $expr")
}

private fun semanticConstantDescriptor(typeName: String): String = when (typeName) {
    "byte" -> "B"
    "short" -> "S"
    "int" -> "I"
    "long" -> "J"
    "char" -> "C"
    else -> throw IllegalArgumentException("semantic constants must use byte, short, int, long, or char; got: $typeName")
}

private fun validateSemanticConstantRange(desc: String, value: Long) {
    val valid = when (desc) {
        "B" -> value in Byte.MIN_VALUE..Byte.MAX_VALUE
        "S" -> value in Short.MIN_VALUE..Short.MAX_VALUE
        "C" -> value in Char.MIN_VALUE.code.toLong()..Char.MAX_VALUE.code.toLong()
        "I" -> value in Int.MIN_VALUE..Int.MAX_VALUE
        "J" -> true
        else -> false
    }
    require(valid) { "constant value $value does not fit descriptor $desc" }
}

private fun hasIgnoredClassAnnotation(classDecl: ClassOrInterfaceDeclaration): Boolean =
    classDecl.annotations.any { annotation ->
        annotation.nameAsString.substringAfterLast('.') in ignoredClassAnnotations
    }

private fun hasExternalClassAnnotation(classDecl: ClassOrInterfaceDeclaration): Boolean =
    annotationNamed(classDecl.annotations, "External") != null

private fun classHeaderSnippet(parsedClass: ParsedMapClass, lines: List<String>): String =
    rangeSlice(lines, parsedClass.lineNo, parsedClass.endLine).substringBefore('{')

private fun memberOrigin(file: Path, lines: List<String>, lineNo: Int): MappingOrigin {
    val line = lines.getOrNull(lineNo - 1)?.trim().orEmpty()
    return MappingOrigin(file, lineNo, line.ifBlank { null })
}

private data class MemberSource(
    val beginLine: Int,
    val snippet: String,
    val origin: MappingOrigin,
)

private fun memberSource(member: NodeWithRange<*>, file: Path, lines: List<String>): MemberSource {
    val range = member.range.orElse(null)
    val beginLine = range?.begin?.line ?: 1
    val endLine = range?.end?.line ?: beginLine
    return MemberSource(
        beginLine = beginLine,
        snippet = rangeSlice(lines, beginLine, endLine),
        origin = memberOrigin(file, lines, beginLine),
    )
}

private fun resolveReadableOwnerExpr(
    cu: CompilationUnit,
    classDecl: ClassOrInterfaceDeclaration,
): String {
    val packageName = if (cu.packageDeclaration.isPresent) cu.packageDeclaration.get().nameAsString else null
    return if (packageName.isNullOrBlank()) classDecl.nameAsString else "$packageName.${classDecl.nameAsString}"
}

private data class ParsedFieldMember(
    val sig: FieldSig,
    val newName: String,
)

private data class ParsedMethodMember(
    val sig: MethodSig,
    val newName: String,
    val paramNames: List<String>,
)

private fun parameterDescriptors(
    parameters: List<Parameter>,
    resolution: TypeDescriptorResolution,
): List<String> = parameters.map { parameter ->
    typeNodeToDescriptor(
        parameter.type,
        resolution,
        allowVoid = false,
        extraArrayDimensions = if (parameter.isVarArgs) 1 else 0,
    )
}

private fun parseExternalCallableMember(
    owner: String,
    name: String,
    returnDesc: String,
    parameters: List<Parameter>,
    sourceSnippet: String,
    resolution: TypeDescriptorResolution,
): ParsedMethodMember {
    require(extractWasMemberComment(sourceSnippet) == null) { "@External members must use their bytecode name directly" }
    val argDescs = parameterDescriptors(parameters, resolution)
    return ParsedMethodMember(
        MethodSig(owner, name, "(${argDescs.joinToString("")})$returnDesc"),
        name,
        parameters.map { it.nameAsString },
    )
}

private data class MemberMappingTables(
    val fields: MutableMap<FieldSig, String> = linkedMapOf(),
    val methods: MutableMap<MethodSig, String> = linkedMapOf(),
    val methodArgs: MutableMap<MethodSig, List<String>> = linkedMapOf(),
    val fieldOrigins: MutableMap<FieldSig, MappingOrigin> = linkedMapOf(),
    val methodOrigins: MutableMap<MethodSig, MappingOrigin> = linkedMapOf(),
    val issues: MutableList<ValidationIssue>,
)

private fun parseFieldMember(
    ownerObf: String,
    member: FieldDeclaration,
    sourceSnippet: String,
    resolution: TypeDescriptorResolution,
): ParsedFieldMember {
    val oldName = extractWasMemberComment(sourceSnippet)
        ?: throw IllegalArgumentException("missing '/* was <obfName> */' comment")

    val variable = member.variables.singleOrNull()
        ?: throw IllegalArgumentException("field declaration must contain exactly one variable")
    val newName = variable.nameAsString
    require(isValidIdentifier(newName)) { "invalid mapped field name: $newName" }

    val fieldDesc = typeNodeToDescriptor(variable.type, resolution, allowVoid = false)
    return ParsedFieldMember(FieldSig(ownerObf, oldName, fieldDesc), newName)
}

private fun parseExternalFieldMember(
    owner: String,
    member: FieldDeclaration,
    sourceSnippet: String,
    resolution: TypeDescriptorResolution,
): ParsedFieldMember {
    require(extractWasMemberComment(sourceSnippet) == null) { "@External members must use their bytecode name directly" }
    val variable = member.variables.singleOrNull()
        ?: throw IllegalArgumentException("field declaration must contain exactly one variable")
    val name = variable.nameAsString
    require(isValidIdentifier(name)) { "invalid external field name: $name" }
    return ParsedFieldMember(FieldSig(owner, name, typeNodeToDescriptor(variable.type, resolution, allowVoid = false)), name)
}

private fun parseMethodMember(
    ownerObf: String,
    member: MethodDeclaration,
    sourceSnippet: String,
    resolution: TypeDescriptorResolution,
): ParsedMethodMember {
    val oldName = extractWasMemberComment(sourceSnippet)
        ?: throw IllegalArgumentException("missing '/* was <obfName> */' comment")

    val newName = member.nameAsString
    require(isValidIdentifier(newName)) { "invalid mapped method name: $newName" }

    val retDesc = typeNodeToDescriptor(member.type, resolution, allowVoid = true)

    val argDescs = parameterDescriptors(member.parameters, resolution)
    val methodDesc = "(${argDescs.joinToString("")})$retDesc"
    return ParsedMethodMember(MethodSig(ownerObf, oldName, methodDesc), newName, member.parameters.map { it.nameAsString })
}

private fun parseExternalMethodMember(
    owner: String,
    member: MethodDeclaration,
    sourceSnippet: String,
    resolution: TypeDescriptorResolution,
): ParsedMethodMember {
    val name = member.nameAsString
    require(isValidIdentifier(name)) { "invalid external method name: $name" }
    val retDesc = typeNodeToDescriptor(member.type, resolution, allowVoid = true)
    return parseExternalCallableMember(owner, name, retDesc, member.parameters, sourceSnippet, resolution)
}

private fun parseConstructorMember(
    owner: String,
    member: ConstructorDeclaration,
    resolution: TypeDescriptorResolution,
): ParsedMethodMember {
    // A constructor may share a line with its class's 'was' comment. Inspect
    // only its own tokens, not the whole source line used for diagnostics.
    require(extractWasMemberComment(member.tokenRange.orElseThrow().toString()) == null) { "constructors do not use a 'was' member comment" }
    require(member.body.statements.isEmpty()) { "mapping constructors require an empty body" }
    val argDescs = parameterDescriptors(member.parameters, resolution)
    return ParsedMethodMember(MethodSig(owner, "<init>", "(${argDescs.joinToString("")})V"), "<init>",
        member.parameters.map { it.nameAsString })
}

private fun <T> shouldSkipUnavailableBuiltin(
    authority: MapAuthority,
    hasClasspathSymbolIndex: Boolean,
    member: T,
    availableMembers: Collection<T>,
): Boolean =
    // API profiles and vendor implementations are allowed to expose only a
    // subset of a pack; project declarations remain strict validation input.
    authority == MapAuthority.BUILTIN && hasClasspathSymbolIndex && member !in availableMembers

private data class ParsedMemberMapFile(
    val path: Path,
    val lines: List<String>,
    val sourceContext: JavaSourceContext,
    val classes: List<ParsedMapClass>,
    val authority: MapAuthority,
)

private data class MapSource(
    val path: Path,
    val displayPath: String,
    val source: String,
    val authority: MapAuthority,
)

private data class ParsedMapClass(
    val decl: ClassOrInterfaceDeclaration,
    val readableOwner: String,
    val packageName: String,
    val lineNo: Int,
    val endLine: Int,
)

private const val maxParseProblemsPerFile = 50

private fun formatMapPath(memberFile: Path, mapsDir: Path): String {
    val normalizedFile = memberFile.toAbsolutePath().normalize()
    val normalizedMapsDir = mapsDir.toAbsolutePath().normalize()
    val relative = try {
        normalizedFile.relativeTo(normalizedMapsDir)
    } catch (_: IllegalArgumentException) {
        null
    }
    return if (relative != null) {
        "${mapsDir.fileName}/${relative.invariantSeparatorsPathString}"
    } else {
        normalizedFile.invariantSeparatorsPathString
    }
}

private fun formatParseProblem(problem: Problem, displayPath: String, lines: List<String>): String {
    val range = problem.location
        .flatMap { it.toRange() }
        .orElse(null)
    val begin = range?.begin
    if (begin == null) {
        return problem.verboseMessage
    }

    val lineNo = begin.line
    val colNo = begin.column
    val sourceLine = lines.getOrNull(lineNo - 1).orEmpty()
    val caretIndent = " ".repeat((colNo - 1).coerceAtLeast(0))
    val message = problem.message.trim()

    return buildString {
        append("$displayPath:$lineNo:$colNo: $message")
        appendLine()
        append("  | $sourceLine")
        appendLine()
        append("  | ${caretIndent}^")
    }
}

private fun formatParseProblems(problems: List<Problem>, displayPath: String, lines: List<String>): String {
    if (problems.isEmpty()) {
        return "unknown parse error"
    }
    val sorted = problems
        .sortedWith(Problem.PROBLEM_BY_BEGIN_POSITION)
    val rendered = sorted
        .take(maxParseProblemsPerFile)
        .map { formatParseProblem(it, displayPath, lines) }
    val hidden = sorted.size - rendered.size
    val suffix = if (hidden > 0) "\n\n... $hidden more parse problem(s) not shown" else ""
    return rendered.joinToString("\n\n") + suffix
}

private fun parseMemberMapSource(mapSource: MapSource): ParsedMemberMapFile {
    val source = mapSource.source
    val lines = source.lines()
    val cu = try {
        parseCompilationUnit(source)
    } catch (exc: ParseProblemException) {
        val details = formatParseProblems(exc.problems, mapSource.displayPath, lines)
        throw IllegalArgumentException(
            "${mapSource.displayPath}: failed to parse map file as Java syntax\n$details",
            exc,
        )
    }
    val topLevelClasses = cu.types.filterIsInstance<ClassOrInterfaceDeclaration>()
    if (topLevelClasses.isEmpty()) {
        throw IllegalArgumentException("${mapSource.displayPath}: missing class declaration")
    }
    val sourceContext = javaSourceContext(cu)
    val classes = topLevelClasses.map { classDecl ->
        val range = classDecl.range.orElse(null)
        ParsedMapClass(
            decl = classDecl,
            readableOwner = resolveReadableOwnerExpr(cu, classDecl),
            packageName = sourceContext.packageName,
            lineNo = range?.begin?.line ?: 1,
            endLine = range?.end?.line ?: range?.begin?.line ?: 1,
        )
    }
    return ParsedMemberMapFile(mapSource.path, lines, sourceContext, classes, mapSource.authority)
}

private fun resolveClassOwnerRaw(
    ownerRaw: String,
    parsedClass: ParsedMapClass,
    knownObfClasses: Set<String>,
): String {
    val directOwner = readableClassToInternal(ownerRaw)
    if ('/' in directOwner || directOwner in knownObfClasses || parsedClass.packageName.isBlank()) {
        return directOwner
    }

    val packageRelativeOwner = readableClassToInternal("${parsedClass.packageName}.$ownerRaw")
    return if (packageRelativeOwner in knownObfClasses) packageRelativeOwner else directOwner
}

private inline fun recordMember(
    member: NodeWithRange<*>,
    memberFile: Path,
    lines: List<String>,
    errors: MutableList<String>,
    block: (MemberSource) -> Unit,
) {
    val source = memberSource(member, memberFile, lines)
    try {
        block(source)
    } catch (exc: IllegalArgumentException) {
        errors += "$memberFile:${source.beginLine}: ${exc.message}"
    }
}

private fun MemberMappingTables.recordField(ownerObf: String, parsedField: ParsedFieldMember, source: MemberSource) {
    val fieldSig = parsedField.sig
    val newName = parsedField.newName
    val existing = fields[fieldSig]
    if (existing != null && existing != newName) {
        issues += ValidationIssue(
            title = "duplicate field mapping with conflicting target for $ownerObf.${fieldSig.name} ${fieldSig.desc}",
            notes = listOf("existing target: $existing", "new target: $newName"),
            hint = "keep a single target name for this obfuscated field mapping",
            origins = listOfNotNull(fieldOrigins[fieldSig], source.origin),
        )
        return
    }

    fields[fieldSig] = newName
    fieldOrigins[fieldSig] = source.origin
}

private fun MemberMappingTables.recordMethod(ownerObf: String, parsedMethod: ParsedMethodMember, source: MemberSource) {
    val methodSig = parsedMethod.sig
    val newName = parsedMethod.newName
    val params = parsedMethod.paramNames
    val existing = methods[methodSig]
    val previousOrigin = methodOrigins[methodSig]
    if (existing != null && existing != newName) {
        issues += ValidationIssue(
            title = "duplicate method mapping with conflicting target for $ownerObf.${methodSig.name}${methodSig.desc}",
            notes = listOf("existing target: $existing", "new target: $newName"),
            hint = "keep a single target name for this obfuscated method mapping",
            origins = listOfNotNull(previousOrigin, source.origin),
        )
        return
    }

    val previousParams = methodArgs[methodSig]
    if (previousParams != null && previousParams != params) {
        val prevLabel = previousParams.joinToString(", ")
        val newLabel = params.joinToString(", ")
        issues += ValidationIssue(
            title = "duplicate method mapping with conflicting parameter names for $ownerObf.${methodSig.name}${methodSig.desc}",
            notes = listOf("existing parameter names: [$prevLabel]", "new parameter names: [$newLabel]"),
            hint = "keep a single parameter-name mapping for this obfuscated method",
            origins = listOfNotNull(previousOrigin, source.origin),
        )
        return
    }

    methods[methodSig] = newName
    methodOrigins[methodSig] = source.origin
    methodArgs[methodSig] = params
}

fun loadJavaLikeMappings(
    mapsDir: Path,
    knownProjectClasses: Set<String>,
    knownClasspathClasses: Set<String> = emptySet(),
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
    includeSemanticMappings: Boolean = true,
): ProjectMappings {
    val legacyClassesMap = mapsDir.resolve("classes.map")
    require(!legacyClassesMap.exists()) {
        "Unsupported legacy mapping file: $legacyClassesMap\n" +
            "Move each class owner into its class declaration: class ReadableName /* was obfuscated/Owner */ { ... }"
    }

    val classes = linkedMapOf<String, String>()
    val readableToObf = linkedMapOf<String, String>()
    val ignoredClasses = linkedSetOf<String>()
    val ownerByClass = linkedMapOf<Pair<Path, Int>, String>()
    val missingOwnerClasses = linkedSetOf<Pair<Path, Int>>()
    val ignoredClassKeys = linkedSetOf<Pair<Path, Int>>()
    val externalClassKeys = linkedSetOf<Pair<Path, Int>>()
    val errors = mutableListOf<String>()
    val issues = mutableListOf<ValidationIssue>()
    val memberMappings = MemberMappingTables(issues = issues)

    val memberFiles = mapsDir.listDirectoryEntries("*.map").sortedBy { it.name }
    val mapSources = buildList {
        if (includeSemanticMappings) {
            loadBuiltinSemanticMapSources(knownClasspathClasses).forEach { builtin ->
                add(MapSource(builtin.path, builtin.displayPath, builtin.source, MapAuthority.BUILTIN))
            }
        }
        memberFiles.forEach { memberFile ->
            add(
                MapSource(
                    path = memberFile,
                    displayPath = formatMapPath(memberFile, mapsDir),
                    source = memberFile.readText(),
                    authority = MapAuthority.PROJECT,
                ),
            )
        }
    }

    val parsedMemberFiles = mutableListOf<ParsedMemberMapFile>()
    for (mapSource in mapSources) {
        try {
            parsedMemberFiles += parseMemberMapSource(mapSource)
        } catch (exc: IllegalArgumentException) {
            errors += exc.message ?: "${mapSource.displayPath}: failed to parse map file as Java syntax"
        }
    }

    val semanticDomains = linkedMapOf<String, SemanticDomain>()
    if (includeSemanticMappings) {
        for (parsed in parsedMemberFiles) {
            for (parsedClass in parsed.classes) {
                val kind = semanticDomainKind(parsedClass.decl) ?: continue
                if (hasExternalClassAnnotation(parsedClass.decl)) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: a semantic domain cannot also be @External"
                }
                if (!parsedClass.decl.isInterface) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: semantic domain '${parsedClass.readableOwner}' must be an interface"
                    continue
                }
                val previous = semanticDomains.putIfAbsent(
                    parsedClass.readableOwner,
                    SemanticDomain(parsedClass.readableOwner, kind),
                )
                if (previous != null) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: duplicate semantic domain: ${parsedClass.readableOwner}"
                }
            }
        }
    }

    val semanticBuilder = SemanticMapBuilder(semanticDomains)
    if (includeSemanticMappings) {
        for (parsed in parsedMemberFiles) {
            for (parsedClass in parsed.classes) {
                val kind = semanticDomainKind(parsedClass.decl) ?: continue
                val values = mutableListOf<SyntheticSemanticValue>()
                val strings = mutableListOf<SemanticStringValue>()
                for (member in parsedClass.decl.members) {
                    if (member !is FieldDeclaration) {
                        errors += "${parsed.path}:${parsedClass.lineNo}: semantic domains may only declare constant fields"
                        continue
                    }
                    for (variable in member.variables) {
                        try {
                            val initializer = variable.initializer.orElseThrow {
                                IllegalArgumentException("synthetic semantic constant '${variable.nameAsString}' requires an initializer")
                            }
                            if (kind == SemanticDomainKind.STRING) {
                                require(variable.typeAsString in setOf("String", "java.lang.String") && initializer.isStringLiteralExpr) {
                                    "String domains require String literal constants"
                                }
                                strings += SemanticStringValue(variable.nameAsString, initializer.asStringLiteralExpr().asString())
                                continue
                            }
                            val desc = semanticConstantDescriptor(variable.typeAsString)
                            val value = parseIntegralConstant(initializer)
                            validateSemanticConstantRange(desc, value)
                            val elementDomain = annotationNamed(member.annotations, "SlotValue")?.let {
                                semanticBuilder.resolveDomain(annotationClassName(it), parsed.sourceContext)
                            }
                            values += SyntheticSemanticValue(variable.nameAsString, desc, value, elementDomain)
                        } catch (exc: IllegalArgumentException) {
                            errors += "${parsed.path}:${variable.range.map { it.begin.line }.orElse(parsedClass.lineNo)}: ${exc.message}"
                        }
                    }
                }
                val duplicateNames = values.groupBy { it.name }.filterValues { it.size > 1 }.keys
                val duplicateValues = values.groupBy { it.value }.filterValues { it.size > 1 }.keys
                if (duplicateNames.isNotEmpty()) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: duplicate semantic constant names in ${parsedClass.readableOwner}: ${duplicateNames.joinToString()}"
                }
                if (duplicateValues.isNotEmpty()) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: duplicate semantic values in ${parsedClass.readableOwner}: ${duplicateValues.joinToString()}"
                }
                val masks = try {
                    val annotation = annotationNamed(parsedClass.decl.annotations, "FlagDomain")
                    val expression = annotation?.let { annotationValue(it, "exclusiveMasks") }
                    val expressions = if (expression is com.github.javaparser.ast.expr.ArrayInitializerExpr) expression.values.toList()
                        else listOfNotNull(expression)
                    expressions.map(::parseIntegralConstant)
                } catch (exc: IllegalArgumentException) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: ${exc.message}"
                    emptyList()
                }
                try {
                    semanticDomains[parsedClass.readableOwner] = completeSemanticDomain(
                        SemanticDomain(parsedClass.readableOwner, kind, values, masks, syntheticStrings = strings),
                        parsedClass.decl.annotations, parsed.sourceContext, semanticBuilder,
                    )
                } catch (exc: IllegalArgumentException) {
                    errors += "${parsed.path}:${parsedClass.lineNo}: ${exc.message}"
                }
            }
        }
    }

    for (parsed in parsedMemberFiles) {
        val memberFile = parsed.path
        val lines = parsed.lines
        for (parsedClass in parsed.classes) {
            if (semanticDomainKind(parsedClass.decl) != null) {
                continue
            }
            val lineNo = parsedClass.lineNo
            val classKey = memberFile to lineNo
            val alreadyMapped = hasIgnoredClassAnnotation(parsedClass.decl)
            val external = hasExternalClassAnnotation(parsedClass.decl)
            val inlineOwner = extractWasClassOwnerComment(classHeaderSnippet(parsedClass, lines))

            if (external) {
                if (alreadyMapped) {
                    errors += "$memberFile:$lineNo: @External and @AlreadyMapped cannot be combined"
                }
                if (!inlineOwner.isNullOrBlank()) {
                    errors += "$memberFile:$lineNo: @External classes must use their bytecode name directly"
                }
                val owner = readableClassToInternal(parsedClass.readableOwner)
                if (owner !in knownClasspathClasses) {
                    errors += "$memberFile:$lineNo: @External class not found in the API classpath: $owner"
                }
                val previous = readableToObf.putIfAbsent(parsedClass.readableOwner, owner)
                if (previous != null && previous != owner) {
                    errors += "$memberFile:$lineNo: external class name conflicts with a project mapping: ${parsedClass.readableOwner}"
                }
                ownerByClass[classKey] = owner
                externalClassKeys += classKey
                continue
            }

            if (alreadyMapped) {
                if (!inlineOwner.isNullOrBlank()) {
                    errors += "$memberFile:$lineNo: @AlreadyMapped classes must not also declare '/* was ... */'"
                }
                if (parsedClass.decl.members.isNotEmpty()) {
                    errors += "$memberFile:$lineNo: @AlreadyMapped classes must not declare member mappings"
                }
                registerIgnoredClassMapping(
                    readableName = parsedClass.readableOwner,
                    knownProjectClasses = knownProjectClasses,
                    classes = classes,
                    readableToObf = readableToObf,
                    ignoredClasses = ignoredClasses,
                    errors = errors,
                    source = "$memberFile:$lineNo",
                )
                ignoredClassKeys += classKey
                continue
            }

            val ownerRaw = when {
                !inlineOwner.isNullOrBlank() -> resolveClassOwnerRaw(inlineOwner, parsedClass, knownProjectClasses)
                readableClassToInternal(parsedClass.readableOwner) in knownProjectClasses -> parsedClass.readableOwner
                else -> null
            }
            if (ownerRaw == null) {
                errors += "$memberFile:$lineNo: missing class owner mapping; add '/* was <obfOwner> */' on class declaration"
                missingOwnerClasses += classKey
                continue
            }
            registerClassMapping(
                readableName = parsedClass.readableOwner,
                obfOwnerRaw = ownerRaw,
                classes = classes,
                readableToObf = readableToObf,
                ignoredClasses = ignoredClasses,
                errors = errors,
                source = "$memberFile:$lineNo",
            )
            ownerByClass[classKey] = readableClassToInternal(ownerRaw)
        }
    }

    addDefaultPackageAliases(readableToObf, errors, mapsDir.toString())

    for (parsed in parsedMemberFiles) {
        val memberFile = parsed.path
        val lines = parsed.lines
        val resolution = typeDescriptorResolution(
            readableToObf = readableToObf,
            knownProjectClasses = knownProjectClasses,
            knownClasspathClasses = knownClasspathClasses,
            sourceContext = parsed.sourceContext,
            fallbackToInferredInternalName = false,
        )
        for (parsedClass in parsed.classes) {
            if (semanticDomainKind(parsedClass.decl) != null) {
                continue
            }
            val classKey = memberFile to parsedClass.lineNo
            if (classKey in ignoredClassKeys) {
                continue
            }
            if (classKey in missingOwnerClasses) {
                continue
            }
            val ownerObf = ownerByClass[classKey] ?: run {
                errors += "$memberFile:${parsedClass.lineNo}: unresolved owner for '${parsedClass.readableOwner}'"
                continue
            }
            val external = classKey in externalClassKeys

            for (member in parsedClass.decl.members) {
                when (member) {
                    is FieldDeclaration -> {
                        recordMember(
                            member = member,
                            memberFile = memberFile,
                            lines = lines,
                            errors = errors,
                        ) { source ->
                            val parsedField = if (external) {
                                parseExternalFieldMember(ownerObf, member, source.snippet, resolution)
                            } else {
                                parseFieldMember(ownerObf, member, source.snippet, resolution).also {
                                    memberMappings.recordField(ownerObf, it, source)
                                }
                            }
                            if (shouldSkipUnavailableBuiltin(
                                    parsed.authority,
                                    classpathSymbolsByClass.isNotEmpty(),
                                    parsedField.sig,
                                    classpathSymbolsByClass[ownerObf]?.fields.orEmpty(),
                                )) {
                                return@recordMember
                            }
                            if (includeSemanticMappings) {
                                annotationNamed(member.annotations, "DomainValue")?.let {
                                    semanticBuilder.bindRealValue(
                                        parsedField.sig,
                                        semanticBuilder.resolveDomain(annotationClassName(it), parsed.sourceContext),
                                        parsed.authority,
                                    )
                                }
                                val target = SemanticTarget.Field(parsedField.sig)
                                semanticBuilder.bindDeclarationSemantics(
                                    target,
                                    Type.getType(parsedField.sig.desc),
                                    member.annotations,
                                    parsed.sourceContext,
                                    parsed.authority,
                                )
                            }
                        }
                    }

                    is MethodDeclaration -> {
                        recordMember(
                            member = member,
                            memberFile = memberFile,
                            lines = lines,
                            errors = errors,
                        ) { source ->
                            val parsedMethod = if (external) {
                                parseExternalMethodMember(ownerObf, member, source.snippet, resolution)
                            } else {
                                parseMethodMember(ownerObf, member, source.snippet, resolution).also {
                                    memberMappings.recordMethod(ownerObf, it, source)
                                }
                            }
                            if (shouldSkipUnavailableBuiltin(
                                    parsed.authority,
                                    classpathSymbolsByClass.isNotEmpty(),
                                    parsedMethod.sig,
                                    classpathSymbolsByClass[ownerObf]?.methods.orEmpty(),
                                )) {
                                return@recordMember
                            }
                            if (includeSemanticMappings) {
                                semanticBuilder.bindCallableSemantics(
                                    parsedMethod.sig,
                                    member.annotations,
                                    member.parameters,
                                    parsed.sourceContext,
                                    parsed.authority,
                                )
                            }
                        }
                    }

                    is ConstructorDeclaration -> {
                        recordMember(
                            member = member,
                            memberFile = memberFile,
                            lines = lines,
                            errors = errors,
                        ) { source ->
                            require(member.nameAsString == parsedClass.decl.nameAsString) { "constructor name must match its mapped class" }
                            val parsedMethod = parseConstructorMember(
                                ownerObf,
                                member,
                                resolution,
                            )
                            if (!external) memberMappings.recordMethod(ownerObf, parsedMethod, source)
                            if (shouldSkipUnavailableBuiltin(
                                    parsed.authority,
                                    classpathSymbolsByClass.isNotEmpty(),
                                    parsedMethod.sig,
                                    classpathSymbolsByClass[ownerObf]?.methods.orEmpty(),
                                )) {
                                return@recordMember
                            }
                            if (includeSemanticMappings) {
                                semanticBuilder.bindCallableSemantics(
                                    method = parsedMethod.sig,
                                    returnAnnotations = member.annotations,
                                    parameters = member.parameters,
                                    context = parsed.sourceContext,
                                    authority = parsed.authority,
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    if (errors.isNotEmpty()) {
        throw IllegalArgumentException(errors.joinToString("\n"))
    }
    if (issues.isNotEmpty()) {
        throw MappingValidationException(issues, mapsDir)
    }

    val canonical = CanonicalMap(
        classes = classes,
        fields = memberMappings.fields,
        methods = memberMappings.methods,
        methodArgs = memberMappings.methodArgs,
        fieldOrigins = memberMappings.fieldOrigins,
        methodOrigins = memberMappings.methodOrigins,
        ignoredClasses = ignoredClasses,
    )
    return ProjectMappings(canonical, semanticBuilder.build())
}

fun loadJavaLikeMap(
    mapsDir: Path,
    knownProjectClasses: Set<String>,
    knownClasspathClasses: Set<String> = emptySet(),
    classpathSymbolsByClass: Map<String, ClassSymbols> = emptyMap(),
): CanonicalMap = loadJavaLikeMappings(
    mapsDir,
    knownProjectClasses,
    knownClasspathClasses,
    classpathSymbolsByClass,
).canonical

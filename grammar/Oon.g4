// SPDX-License-Identifier: MPL-2.0

grammar Oon;

schemaDocument
    : (typeDeclaration | schemaDeclaration)* EOF
    ;

typeDeclaration
    : TYPE name EQUAL typeExpression SEMI
    ;

schemaDeclaration
    : SCHEMA name EQUAL typeExpression SEMI
    ;

overlayDocument
    : schemaLocator overlayDeclaration* EOF
    ;

schemaLocator
    : SCHEMA EQUAL STRING SEMI
    ;

overlayDeclaration
    : OVERLAY name EQUAL block SEMI
    ;

typeExpression
    : keyedType (PIPE keyedType)*
    ;

keyedType
    : primaryType (KEY name)?
    ;

primaryType
    : primitiveType
    | literalType
    | name
    | objectType
    | listType
    | mapType
    | tupleType
    | taggedType
    | LPAREN typeExpression RPAREN
    ;

primitiveType
    : STRING_KW | INT_KW | FLOAT_KW | BOOL_KW | ANY
    ;

literalType
    : STRING | signedNumber | TRUE | FALSE
    ;

signedNumber
    : MINUS? (INT | FLOAT)
    ;

objectType
    : LBRACE fieldDeclaration* RBRACE
    ;

fieldDeclaration
    : name QUESTION? EQUAL typeExpression SEMI
    ;

listType
    : LIST LT typeExpression GT
    ;

mapType
    : MAP LT typeExpression GT
    ;

tupleType
    : TUPLE LT (typeExpression SEMI)* GT
    ;

taggedType
    : TAGGED LBRACE
      TAG EQUAL name SEMI
      (COMMON EQUAL objectShape SEMI)?
      VARIANTS EQUAL variantBlock SEMI
      RBRACE
    ;

objectShape
    : objectType | name
    ;

variantBlock
    : LBRACE variantDeclaration+ RBRACE
    ;

variantDeclaration
    : name EQUAL objectShape SEMI
    ;

block
    : LBRACE statement* RBRACE
    ;

statement
    : action | conditional | loop
    ;

action
    : path EQUAL expression SEMI
    | MERGE path EQUAL expression SEMI
    | SET path EQUAL expression SEMI
    | RESET path SEMI
    ;

conditional
    : IF expression block (ELSE IF expression block)* (ELSE block)? SEMI
    ;

loop
    : FOR name IN expression block SEMI
    ;

path
    : DOT (pathSegment (DOT pathSegment)*)?
    ;

pathSegment
    : name | INT | STRING
    ;

expression
    : orExpression
    ;

orExpression
    : andExpression (OR andExpression)*
    ;

andExpression
    : equalityExpression (AND equalityExpression)*
    ;

equalityExpression
    : comparisonExpression (EQEQ comparisonExpression)*
    ;

comparisonExpression
    : additiveExpression ((LT | GT | LE | GE) additiveExpression)*
    ;

additiveExpression
    : multiplicativeExpression ((PLUS | MINUS) multiplicativeExpression)*
    ;

multiplicativeExpression
    : unaryExpression ((STAR | SLASH) unaryExpression)*
    ;

unaryExpression
    : (NOT | MINUS) unaryExpression | primaryExpression
    ;

primaryExpression
    : STRING | INT | FLOAT | TRUE | FALSE
    | path
    | name
    | objectLiteral
    | listLiteral
    | tupleLiteral
    | LPAREN expression RPAREN
    ;

objectLiteral
    : LBRACE relativeAssignment* RBRACE
    ;

relativeAssignment
    : relativePath EQUAL expression SEMI
    ;

relativePath
    : (name | STRING) (DOT pathSegment)*
    ;

listLiteral
    : LBRACK (expression SEMI)* RBRACK
    ;

tupleLiteral
    : LPAREN (expression SEMI)* RPAREN
    ;

// Lowercase keywords are contextual: the parser's name rule admits them where
// a name is unambiguous. Mixed-case spellings are NAME tokens.
name
    : NAME | AND | BOOL_KW | COMMON | ELSE | FALSE | FLOAT_KW | FOR | IF | IN
    | INT_KW | KEY | LIST | MAP | MERGE | NOT | OR | OVERLAY | RESET | SCHEMA
    | SET | STRING_KW | TAG | TAGGED | TRUE | TUPLE | TYPE | VARIANTS | ANY
    ;

AND: 'and';
BOOL_KW: 'bool';
COMMON: 'common';
ELSE: 'else';
FALSE: 'false';
FLOAT_KW: 'float';
FOR: 'for';
IF: 'if';
IN: 'in';
INT_KW: 'int';
KEY: 'key';
LIST: 'list';
MAP: 'map';
MERGE: 'merge';
NOT: 'not';
OR: 'or';
OVERLAY: 'overlay';
RESET: 'reset';
SCHEMA: 'schema';
SET: 'set';
STRING_KW: 'string';
TAG: 'tag';
TAGGED: 'tagged';
TRUE: 'true';
TUPLE: 'tuple';
TYPE: 'type';
VARIANTS: 'variants';
ANY: 'any';

LE: '<=';
GE: '>=';
EQEQ: '==';
EQUAL: '=';
LT: '<';
GT: '>';
PLUS: '+';
MINUS: '-';
STAR: '*';
SLASH: '/';
PIPE: '|';
QUESTION: '?';
DOT: '.';
SEMI: ';';
LBRACE: '{';
RBRACE: '}';
LBRACK: '[';
RBRACK: ']';
LPAREN: '(';
RPAREN: ')';

FLOAT: DIGIT+ '.' DIGIT+;
INT: DIGIT+;

// At least one non-digit component or one hyphen distinguishes NAME from INT.
NAME
    : [A-Za-z_] [A-Za-z0-9_]* ('-' [A-Za-z0-9_]+)*
    | DIGIT+ [A-Za-z_] [A-Za-z0-9_]* ('-' [A-Za-z0-9_]+)*
    | DIGIT+ ('-' [A-Za-z0-9_]+)+
    ;

STRING
    : '"""' (ESCAPE | .)*? '"""'
    | '"' (ESCAPE | ~["\\\r\n])* '"'
    ;

fragment ESCAPE
    : '\\' (["\\nrt] | 'u' HEX HEX HEX HEX)
    ;
fragment HEX: [0-9a-fA-F];
fragment DIGIT: [0-9];

COMMENT: '#' ~[\r\n]* -> skip;
WS: [ \t\r\n]+ -> skip;
